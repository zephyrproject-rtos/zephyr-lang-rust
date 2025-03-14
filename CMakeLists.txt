# SPDX-License-Identifier: Apache-2.0

# Rust make support

set(RUST_MODULE_DIR "${CMAKE_CURRENT_LIST_DIR}" CACHE INTERNAL "")

# Initially, we just have a single DT augment file.
set(DT_AUGMENTS "${CMAKE_CURRENT_LIST_DIR}/dt-rust.yaml" CACHE INTERNAL "")

# Zephyr targets are defined through Kconfig.  We need to map these to
# an appropriate llvm target triple.  This sets `RUST_TARGET` in the
# parent scope, or an error if the target is not yet supported by
# Rust.
function(_rust_map_target)
  # Map Zephyr targets to LLVM targets.
  if(CONFIG_CPU_CORTEX_M)
    if(CONFIG_CPU_CORTEX_M0 OR CONFIG_CPU_CORTEX_M0PLUS OR CONFIG_CPU_CORTEX_M1)
      set(RUST_TARGET "thumbv6m-none-eabi" PARENT_SCOPE)
    elseif(CONFIG_CPU_CORTEX_M3)
      set(RUST_TARGET "thumbv7m-none-eabi" PARENT_SCOPE)
    elseif(CONFIG_CPU_CORTEX_M4 OR CONFIG_CPU_CORTEX_M7)
      if(CONFIG_FP_HARDABI OR FORCE_FP_HARDABI)
        set(RUST_TARGET "thumbv7em-none-eabihf" PARENT_SCOPE)
      else()
        set(RUST_TARGET "thumbv7em-none-eabi" PARENT_SCOPE)
      endif()
    elseif(CONFIG_CPU_CORTEX_M23)
      set(RUST_TARGET "thumbv8m.base-none-eabi" PARENT_SCOPE)
    elseif(CONFIG_CPU_CORTEX_M33 OR CONFIG_CPU_CORTEX_M55)
      # Not a typo, Zephyr, uses ARMV7_M_ARMV8_M_FP to select the FP even on v8m.
      if(CONFIG_FP_HARDABI OR FORCE_FP_HARDABI)
        set(RUST_TARGET "thumbv8m.main-none-eabihf" PARENT_SCOPE)
      else()
        set(RUST_TARGET "thumbv8m.main-none-eabi" PARENT_SCOPE)
      endif()

      # Todo: The M55 is thumbv8.1m.main-none-eabi, which can be added when Rust
      # gain support for this target.
    else()
      message(FATAL_ERROR "Unknown Cortex-M target.")
    endif()
  elseif(CONFIG_RISCV)
    if(CONFIG_RISCV_ISA_RV64I)
      # TODO: Should fail if the extensions don't match.
      set(RUST_TARGET "riscv64imac-unknown-none-elf" PARENT_SCOPE)
    elseif(CONFIG_RISCV_ISA_RV32I)
      # TODO: We have multiple choices, try to pick the best.
      set(RUST_TARGET "riscv32i-unknown-none-elf" PARENT_SCOPE)
    else()
      message(FATAL_ERROR "Rust: Unsupported riscv ISA")
    endif()
  elseif(CONFIG_CPU_CORTEX_R52)
    set(RUST_TARGET "armv8r-none-eabihf" PARENT_SCOPE)

    # build core (and maybe also alloc) from source for tier 3 target
    if(CONFIG_RUST_ALLOC)
      set(CARGO_EXTRA_FLAGS -Z build-std=core,alloc PARENT_SCOPE)
    else()
      set(CARGO_EXTRA_FLAGS -Z build-std=core PARENT_SCOPE)
    endif()
  else()
    message(FATAL_ERROR "Rust: Add support for other target")
  endif()
endfunction()

function(get_include_dirs target dirs)
  get_target_property(include_dirs ${target} INTERFACE_INCLUDE_DIRECTORIES)
  if(include_dirs)
    set(${dirs} "${include_dirs}" PARENT_SCOPE)
  else()
    set(${dirs} "" PARENT_SCOPE)
  endif()
endfunction()

function(rust_cargo_application)
  # For now, hard-code the Zephyr crate directly here.  Once we have
  # more than one crate, these should be added by the modules
  # themselves.
  set(LIB_RUST_CRATES zephyr zephyr-build zephyr-sys)

  get_include_dirs(zephyr_interface include_dirs)

  get_property(include_defines TARGET zephyr_interface PROPERTY INTERFACE_COMPILE_DEFINITIONS)
  message(STATUS "Includes: ${include_dirs}")
  message(STATUS "Defines: ${include_defines}")

  _rust_map_target()
  message(STATUS "Building Rust llvm target ${RUST_TARGET}")

  # TODO: Make sure RUSTFLAGS is not set.

  # choose debug/release build based on Kconfig choice
  if(CONFIG_RUST_CARGO_PROFILE_RELEASE)
    message(STATUS "Cargo build profile: release")
    set(RUST_BUILD_TYPE release)
  else()
    message(STATUS "Cargo build profile: dev")
    set(RUST_BUILD_TYPE debug)
  endif()
  set(BUILD_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/${RUST_TARGET}/${RUST_BUILD_TYPE}")

  set(CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/rust/target")
  set(RUST_LIBRARY "${CARGO_TARGET_DIR}/${RUST_TARGET}/${RUST_BUILD_TYPE}/librustapp.a")
  set(SAMPLE_CARGO_CONFIG "${CMAKE_CURRENT_BINARY_DIR}/rust/sample-cargo-config.toml")

  # The generated C binding wrappers. These are bindgen-generated wrappers for the inline functions
  # within Zephyr.
  set(WRAPPER_FILE "${CMAKE_CURRENT_BINARY_DIR}/rust/wrapper.c")

  # To get cmake to always invoke Cargo requires a bit of a trick.  We make the output of the
  # command a file that never gets created.  This will cause cmake to always rerun cargo.  We
  # add the actual library as a BYPRODUCTS list of this command, otherwise, the first time the
  # link will fail because it doesn't think it knows how to build the library.  This will also
  # cause the relink when the cargo command actually does rebuild the rust code.
  set(DUMMY_FILE "${CMAKE_BINARY_DIR}/always-run-cargo.dummy")

  # For each module in zephyr-rs, add entry both to the .cargo/config template and for the
  # command line, since either invocation will need to see these.
  set(command_paths)
  set(config_paths "")
  message(STATUS "Processing crates: ${ZEPHYR_RS_MODULES}")
  foreach(module IN LISTS LIB_RUST_CRATES)
    message(STATUS "module: ${module}")
    set(config_paths
      "${config_paths}\
${module}.path = \"$CACHE{RUST_MODULE_DIR}/${module}\"
")
    list(APPEND command_paths
      "--config"
      "patch.crates-io.${module}.path=\\\"$CACHE{RUST_MODULE_DIR}/${module}\\\""
      )
  endforeach()

  # Write out a cargo config file that can be copied into `.cargo/config.toml` (or made a
  # symlink) in the source directory to allow various IDE tools and such to work.  The build we
  # invoke will override these settings, in case they are out of date.  Everything set here
  # should match the arguments given to the cargo build command below.
  file(WRITE ${SAMPLE_CARGO_CONFIG} "
# This is a generated sample .cargo/config.toml file from the Zephyr build.
# At the time of generation, this represented the settings needed to allow
# a `cargo build` command to compile the rust code using the current Zephyr build.
# If any settings in the Zephyr build change, this could become out of date.
[build]
target = \"${RUST_TARGET}\"
target-dir = \"${CARGO_TARGET_DIR}\"

[env]
BUILD_DIR = \"${CMAKE_CURRENT_BINARY_DIR}\"
DOTCONFIG = \"${DOTCONFIG}\"
ZEPHYR_DTS = \"${ZEPHYR_DTS}\"
INCLUDE_DIRS = \"${include_dirs}\"
INCLUDE_DEFINES = \"${include_defines}\"
WRAPPER_FILE = \"${WRAPPER_FILE}\"
BINARY_DIR_INCLUDE_GENERATED = \"${BINARY_DIR_INCLUDE_GENERATED}\"
DT_AUGMENTS = \"${DT_AUGMENTS}\"

[patch.crates-io]
${config_paths}
")

  # The block of environment variables below could theoretically be captured in a variable, but this
  # seems "challenging" in CMake (to be polite), as many of these contain spaces, and the quoting
  # rules in CMake are inconsistent, at best.
  # TODO: Figure out how to factor these out.

  # The library is built by invoking Cargo.
  add_custom_command(
    OUTPUT ${DUMMY_FILE}
    BYPRODUCTS ${RUST_LIBRARY} ${WRAPPER_FILE}
    COMMAND
      ${CMAKE_COMMAND} -E
      env BUILD_DIR=${CMAKE_CURRENT_BINARY_DIR}
      ZEPHYR_BASE=${ZEPHYR_BASE}
      DOTCONFIG=${DOTCONFIG}
      ZEPHYR_DTS=${ZEPHYR_DTS}
      INCLUDE_DIRS="${include_dirs}"
      INCLUDE_DEFINES="${include_defines}"
      WRAPPER_FILE="${WRAPPER_FILE}"
      DT_AUGMENTS="${DT_AUGMENTS}"
      BINARY_DIR_INCLUDE_GENERATED="${BINARY_DIR_INCLUDE_GENERATED}"
      cargo build
      --profile $<IF:$<BOOL:${CONFIG_RUST_CARGO_PROFILE_RELEASE}>,release,dev>

      # Override the features according to the shield given. For a general case,
      # this will need to come from a variable or argument.
      # TODO: This needs to be passed in.
      # --no-default-features
      # --features ${SHIELD_FEATURE}

      # Set a replacement so that packages can just use `zephyr-sys` as a package
      # name to find it.
      ${command_paths}
      --target ${RUST_TARGET}
      --target-dir ${CARGO_TARGET_DIR}
      ${CARGO_EXTRA_FLAGS}
    COMMENT "Building Rust application"
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    USES_TERMINAL
  )

  # Be sure we don't try building this until all of the generated headers have been generated.
  add_custom_target(librustapp ALL
    DEPENDS ${DUMMY_FILE}
        # The variables, defined at the top level, don't seem to be accessible here.
        syscall_list_h_target
        driver_validation_h_target
        kobj_types_h_target
  )

  # Command to generate the rust docs.  As mentioned above, the whole environment is duplicated, so
  # it is important to keep this in sync with the above.
  add_custom_command(
    OUTPUT generate_rust_docs
    COMMAND
      ${CMAKE_COMMAND} -E
      env BUILD_DIR=${CMAKE_CURRENT_BINARY_DIR}
      ZEPHYR_BASE=${ZEPHYR_BASE}
      DOTCONFIG=${DOTCONFIG}
      ZEPHYR_DTS=${ZEPHYR_DTS}
      INCLUDE_DIRS="${include_dirs}"
      INCLUDE_DEFINES="${include_defines}"
      WRAPPER_FILE="${WRAPPER_FILE}"
      DT_AUGMENTS="${DT_AUGMENTS}"
      BINARY_DIR_INCLUDE_GENERATED="${BINARY_DIR_INCLUDE_GENERATED}"
      cargo doc
      ${rust_build_type_arg}

      # Override the features according to the shield given. For a general case,
      # this will need to come from a variable or argument.
      # TODO: This needs to be passed in.
      # --no-default-features
      # --features ${SHIELD_FEATURE}

      # Set a replacement so that packages can just use `zephyr-sys` as a package
      # name to find it.
      ${command_paths}
      --target ${RUST_TARGET}
      --target-dir ${CARGO_TARGET_DIR}
    COMMENT "Building Rust documentation"
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
  )

  add_custom_target(rustdoc
    DEPENDS generate_rust_docs
        # The variables, defined at the top level, don't seem to be accessible here.
        syscall_list_h_target
        driver_validation_h_target
        kobj_types_h_target
  )

  # Linking with the <rt_library> (`$<TARGET_PROPERTY:linker,rt_library>`).
  # -lgcc / -lcompiler_rt depending on toolchain, linker, and runtime library configuration.
  # In general this shouldn't be needed, as the runtime libary is generally linked late, but
  # librustapp.a includes it's own runtime functions, and on riscv (and potentially others) an
  # unrecognized / unknown type is used in the relocation section for clzsi2 object.
  # Thus we must for current time ensure that the runtime library is before librustapp.a.
  # Example of warning reported by ld when this fix is not in place:
  # <path>/ld.bfd: rust/target/riscv64imac-unknown-none-elf/debug/librustapp.a(45c91108d938afe8-clzdi2.o): unsupported relocation type 0x3d
  target_link_libraries(app PUBLIC $<TARGET_PROPERTY:linker,rt_library> -Wl,--allow-multiple-definition ${RUST_LIBRARY})
  add_dependencies(app librustapp)

  # Presumably, Rust applications will have no C source files, but cmake will require them.
  # Add an empty file so that this will build.  The main will come from the rust library.
  target_sources(app PRIVATE $CACHE{RUST_MODULE_DIR}/main.c ${WRAPPER_FILE})
endfunction()

# Function to build a Rust library using cargo, to be integrated into a C-based
# Zephyr application.
#
# In contrast to the rust_cargo_application function, which builds an entire
# Zephyr application from primarily Rust source files, this function is designed
# to build a standalone Rust crate into a library that can be integrated into a
# Zephyr application whose source code base is primarily written in C.
#
# A library built with this function will typically provide a C API through a
# Foreign Function Interface (FFI) whose C header files will be automatically
# generated from the Rust source code by the cbindgen tool, therefore use of
# this function requires the cbindgen tool to be installed on the build host.
#
# The caller must specify the following arguments:
#
#   LIBRARY_NAME <library-name> - the name of the library
#   CRATE_PATH <crate-path>     - path to the root of the Rust crate to build
#
# The caller may optionally specify the following argument:
#
#   MEMORY_PARTITION <memory-partition-name>
#
# which, if specified, will place any static global symbols built from the Rust
# sources into the specified memory partition, which can allow the symbols to be
# used from user mode threads.
#
# The function will add the specified library name into the build environment as
# a symbol that can be linked into the target Zephyr application using e.g.
# target_link_libraries.
#
# Usage:
#   rust_cargo_library(LIBRARY_NAME <library-name>
#                      CRATE_PATH <crate-path>
#                      |MEMORY_PARTITION <memory-partition-name>|
#   )
#
function(rust_cargo_library)
  # parse function arguments
  set(flags)
  set(args LIBRARY_NAME CRATE_PATH MEMORY_PARTITION)
  set(listArgs)
  cmake_parse_arguments(arg "${flags}" "${args}" "${listArgs}" ${ARGN})

  # verify required arguments
  if (NOT arg_LIBRARY_NAME)
    message(FATAL_ERROR "[rust_cargo_library]: LIBRARY_NAME is a required argument")
  endif()
  if (NOT arg_CRATE_PATH)
    message(FATAL_ERROR "[rust_cargo_library]: CRATE_PATH is a required argument")
  endif()

  # locate cbindgen executable
  find_program(CBINDGEN_EXE cbindgen REQUIRED)

  _rust_map_target()
  message(STATUS "Building Rust library ${arg_LIBRARY_NAME} for llvm target ${RUST_TARGET}")

  # choose debug/release build based on Kconfig choice
  if(CONFIG_RUST_CARGO_PROFILE_RELEASE)
    message(STATUS "Cargo build profile: release")
    set(RUST_BUILD_TYPE release)
  else()
    message(STATUS "Cargo build profile: dev")
    set(RUST_BUILD_TYPE debug)
  endif()
  set(BUILD_LIB_DIR ${CMAKE_CURRENT_SOURCE_DIR}/${RUST_TARGET}/${RUST_BUILD_TYPE})

  # library filename is based on arg_LIBRARY_NAME
  set(CARGO_TARGET_DIR ${CMAKE_CURRENT_BINARY_DIR}/rust/target)
  set(RUST_LIBRARY ${CARGO_TARGET_DIR}/${RUST_TARGET}/${RUST_BUILD_TYPE}/lib${arg_LIBRARY_NAME}.a)

  # cbindgen header filename is based on arg_LIBRARY_NAME with _ replaced by -
  string(REPLACE "_" "-" CBINDGEN_FILENAME "${arg_LIBRARY_NAME}-cbindgen.h")
  set(CBINDGEN_HEADER ${CMAKE_CURRENT_BINARY_DIR}/${CBINDGEN_FILENAME})

  # The library is built by invoking Cargo.
  add_custom_target(
    ${arg_LIBRARY_NAME}_builder
    BYPRODUCTS ${RUST_LIBRARY} ${CBINDGEN_HEADER}

    # build the library
    COMMAND
      cargo build
      --profile $<IF:$<STREQUAL:${CONFIG_RUST_CARGO_PROFILE_RELEASE},y>,release,dev>
      --target ${RUST_TARGET}
      --target-dir ${CARGO_TARGET_DIR}
      ${CARGO_EXTRA_FLAGS}

    # autogenerate the C header file using cbindgen
    COMMAND
      ${CMAKE_COMMAND}
      -E env ${CBINDGEN_EXE}
      --config cbindgen.toml
      --output ${CBINDGEN_HEADER}
      --lang c

    WORKING_DIRECTORY ${arg_CRATE_PATH}
    USES_TERMINAL
  )

  # Create a library target and add the location of the generated header file
  # to its include path.
  add_library(${arg_LIBRARY_NAME} STATIC IMPORTED GLOBAL)
  add_dependencies(${arg_LIBRARY_NAME} ${arg_LIBRARY_NAME}_builder)
  set_target_properties(${arg_LIBRARY_NAME} PROPERTIES IMPORTED_LOCATION ${RUST_LIBRARY})
  set_target_properties(${arg_LIBRARY_NAME} PROPERTIES INTERFACE_INCLUDE_DIRECTORIES ${CMAKE_CURRENT_BINARY_DIR})

  # Place any statics from the rust library into the specified memory partition.
  if (arg_MEMORY_PARTITION)
    zephyr_append_cmake_library(${arg_LIBRARY_NAME})
    set(ZEPHYR_CURRENT_LIBRARY ${arg_LIBRARY_NAME})
    zephyr_library_app_memory(${arg_MEMORY_PARTITION})
  endif()
endfunction()
