//! Support for augmenting the device tree.
//!
//! There are various aspects of the device tree in Zephyr whose semantics are only indirectly
//! defined by the behavior of C code. Rather than trying to decipher this at build time, we will
//! use one or more yaml files that describe aspects of the device tree.
//!
//! This module is responsible for the format of this config file and the parsed contents will be
//! used to generate the [`Augment`] objects that will do the actual augmentation of the generated
//! device tree.
//!
//! Each augment is described by a top-level yaml element in an array.

use std::{fs::File, path::Path};

use anyhow::Result;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use serde::{Deserialize, Serialize};

use crate::devicetree::{output::dt_to_lower_id, Value, Word};

use super::{DeviceTree, Node};

/// This action is given to each node in the device tree, and it is given a chance to return
/// additional code to be included in the module associated with that entry.  These are all
/// assembled together and included in the final generated devicetree.rs.
pub trait Augment {
    /// The default implementation checks if this node matches and calls a generator if it does, or
    /// does nothing if not.
    fn augment(&self, node: &Node, tree: &DeviceTree) -> TokenStream {
        // If there is a status field present, and it is not set to "okay", don't augment this node.
        if let Some(status) = node.get_single_string("status") {
            if status != "okay" {
                return TokenStream::new();
            }
        }
        if self.is_compatible(node) {
            self.generate(node, tree)
        } else {
            TokenStream::new()
        }
    }

    /// A query if this node is compatible with this augment.  A simple case might check the node's
    /// compatible field, but also makes sense to check a parent's compatible.
    fn is_compatible(&self, node: &Node) -> bool;

    /// A generator to be called when we are compatible.
    fn generate(&self, node: &Node, tree: &DeviceTree) -> TokenStream;
}

/// A top level augmentation.
///
/// This top level augmentation describes how to match a given node within the device tree, and then
/// what kind of action to describe upon that.
#[derive(Debug, Serialize, Deserialize)]
pub struct Augmentation {
    /// A name for this augmentation.  Used for diagnostic purposes.
    name: String,
    /// What to match.  This is an array, and all must match for a given node to be considered.
    /// This does mean that if this is an empty array, it will match on every node.
    rules: Vec<Rule>,
    /// What to do when a given node matches.
    actions: Vec<Action>,
}

impl Augment for Augmentation {
    fn is_compatible(&self, node: &Node) -> bool {
        self.rules.iter().all(|n| n.is_compatible(node))
    }

    fn generate(&self, node: &Node, tree: &DeviceTree) -> TokenStream {
        let name = format_ident!("{}", dt_to_lower_id(&self.name));
        let actions = self.actions.iter().map(|a| a.generate(&name, node, tree));

        quote! {
            #(#actions)*
        }
    }
}

/// A matching rule.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", content = "value")]
pub enum Rule {
    /// A set of "or" matches.
    Or(Vec<Rule>),
    /// A set of "and" matches.  Not needed at the top level, as the top level vec is an implicit
    /// and.
    And(Vec<Rule>),
    /// Matches if the node has the given property.
    HasProp(String),
    /// Matches if this node has one of the listed compatible strings.  The the 'level' property
    /// indicates how many levels up in the tree.  Zero means match the current node, 1 means the
    /// parent node, and so on.
    Compatible {
        names: Vec<String>,
        level: usize,
    },
    /// Matches at the root of tree.
    Root,
}

impl Rule {
    fn is_compatible(&self, node: &Node) -> bool {
        match self {
            Rule::Or(rules) => rules.iter().any(|n| n.is_compatible(node)),
            Rule::And(rules) => rules.iter().all(|n| n.is_compatible(node)),
            Rule::HasProp(name) => node.has_prop(name),
            Rule::Compatible { names, level } => parent_compatible(node, names, *level),
            Rule::Root => node.parent.borrow().is_none(),
        }
    }
}

/// Determine if a node is compatible, looking `levels` levels up in the tree, where 0 means this
/// node.
fn parent_compatible(node: &Node, names: &[String], level: usize) -> bool {
    // Writing this recursively simplifies the borrowing a lot.  Otherwise, we'd have to clone the
    // RCs.  Our choice is the extra clone, or keeping the borrowed values on the stack.  This code
    // runs on the host, so the stack is easier.
    if level == 0 {
        names.iter().any(|n| node.is_compatible(n))
    } else {
        if let Some(parent) = node.parent.borrow().as_ref() {
            parent_compatible(parent, names, level - 1)
        } else {
            false
        }
    }
}

/// An action to perform
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", content = "value")]
pub enum Action {
    /// Generate an "instance" with a specific device name.
    Instance {
        /// Where to get the raw device information.
        raw: RawInfo,
        /// The name of the full path (within the zephyr-sys crate) for the wrapper node for this
        /// device.
        device: String,
        /// A Kconfig option to allow the instances to only be present when said driver is compiled
        /// in.
        kconfig: Option<String>,
    },
    /// Generate a getter for a gpio assignment property.
    GpioPins {
        /// The name of the property holding the pins.
        property: String,
        /// The name of the getter function.
        getter: String,
    },
    /// Generate all of the labels as its own node.
    Labels,
}

impl Action {
    fn generate(&self, _name: &Ident, node: &Node, tree: &DeviceTree) -> TokenStream {
        match self {
            Action::Instance { raw, device, kconfig } => {
                raw.generate(node, device, kconfig.as_deref())
            }
            Action::GpioPins { property, getter } => {
                let upper_getter = getter.to_uppercase();
                let getter = format_ident!("{}", getter);
                // TODO: This isn't actually right, these unique values should be based on the pin
                // definition so that we'll get a compile error if two parts of the DT reference the
                // same pin.

                let pins = node.get_property(property).unwrap();
                let npins = pins.len();

                let uniques: Vec<_> = (0..npins).map(|n| {
                    format_ident!("{}_UNIQUE_{}", upper_getter, n)
                }).collect();

                let pins = pins
                    .iter()
                    .zip(uniques.iter())
                    .map(|(pin, unique)| decode_gpios_gpio(unique, pin));

                let unique_defs = uniques.iter().map(|u| {
                    quote! {
                        static #u: crate::device::Unique = crate::device::Unique::new();
                    }
                });

                quote! {
                    #(#unique_defs)*
                    pub fn #getter() -> [Option<crate::device::gpio::GpioPin>; #npins] {
                        [#(#pins),*]
                    }
                }
            }
            Action::Labels => {
                let nodes = tree.labels.iter().map(|(k, v)| {
                    let name = dt_to_lower_id(k);
                    let path = v.route_to_rust();
                    quote! {
                        pub mod #name {
                            pub use #path::*;
                        }
                    }
                });

                quote! {
                    // This does assume the devicetree doesn't have a "labels" node at the root.
                    pub mod labels {
                        /// All of the labeles in the device tree.  The device tree compiler
                        /// enforces that these are unique, allowing references such as
                        /// `zephyr::devicetree::labels::labelname::get_instance()`.
                        #(#nodes)*
                    }
                }
            }
        }
    }
}

/// Decode a single gpio entry.
fn decode_gpios_gpio(unique: &Ident, entry: &Value) -> TokenStream {
    let entry = if let Value::Words(w) = entry {
        w
    } else {
        panic!("gpios list is not list of <&gpionnn aa bbb>");
    };
    if entry.len() != 3 {
        panic!("gpios currently must be three items");
    }
    let gpio_route = entry[0].get_phandle().unwrap().node_ref().route_to_rust();
    let args: Vec<u32> = entry[1..].iter().map(|n| n.get_number().unwrap()).collect();

    quote! {
        // TODO: Don't hard code this but put in yaml file.
        unsafe {
            crate::device::gpio::GpioPin::new(
                &#unique,
                #gpio_route :: get_instance_raw(),
                #(#args),*)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", content = "value")]
pub enum RawInfo {
    /// Get the raw device directly from this node.
    Myself {
        args: Vec<ArgInfo>,
    },
    /// Get the reference from a parent of this node, at a given level.
    Parent {
        /// How many levels to look up.  0 would refer to this node (but would also be an error).
        level: usize,
        args: Vec<ArgInfo>,
    },
    /// Get the raw device from a phandle property.  Additional parameters in the phandle will be
    /// passed as additional arguments to the `new` constructor on the wrapper type.
    Phandle(String),
}

impl RawInfo {
    fn generate(&self, node: &Node, device: &str, kconfig: Option<&str>) -> TokenStream {
        let device_id = str_to_path(device);
        let kconfig = if let Some(name) = kconfig {
            let name = format_ident!("{}", name);
            quote! {
                #[cfg(#name)]
            }
        } else {
            quote! {}
        };

        match self {
            RawInfo::Myself { args } => {
                let get_args = args.iter().map(|arg| arg.args(node));

                let ord = node.ord;
                let rawdev = format_ident!("__device_dts_ord_{}", ord);
                quote! {
                    /// Get the raw `const struct device *` of the device tree generated node.
                    #kconfig
                    pub unsafe fn get_instance_raw() -> *const crate::raw::device {
                        &crate::raw::#rawdev
                    }

                    #kconfig
                    static UNIQUE: crate::device::Unique = crate::device::Unique::new();
                    #kconfig
                    pub fn get_instance() -> Option<#device_id> {
                        unsafe {
                            let device = get_instance_raw();
                            #device_id::new(&UNIQUE, device, #(#get_args),*)
                        }
                    }
                }
            }
            RawInfo::Phandle(pname) => {
                let words = node.get_words(pname).unwrap();
                // We assume that elt 0 is the phandle, and that the rest are numbers.
                let target = if let Word::Phandle(handle) = &words[0] {
                    handle.node_ref()
                } else {
                    panic!("phandle property {:?} in node is empty", pname);
                };

                // TODO: We would try to correlate with parent node's notion of number of cells, and
                // try to handle cases where there is more than one reference.  It is unclear when
                // this will be needed.
                let args: Vec<u32> = words[1..].iter().map(|n| n.get_number().unwrap()).collect();

                let target_route = target.route_to_rust();

                quote! {
                    #kconfig
                    static UNIQUE: crate::device::Unique = crate::device::Unique::new();
                    #kconfig
                    pub fn get_instance() -> Option<#device_id> {
                        unsafe {
                            let device = #target_route :: get_instance_raw();
                            #device_id::new(&UNIQUE, device, #(#args),*)
                        }
                    }
                }
            }
            RawInfo::Parent { level, args } => {
                let get_args = args.iter().map(|arg| arg.args(node));

                assert!(*level > 0);
                let mut path = quote! {super};
                for _ in 1..*level {
                    path = quote! { #path :: super };
                }

                quote! {
                    #kconfig
                    static UNIQUE: crate::device::Unique = crate::device::Unique::new();
                    #kconfig
                    pub fn get_instance() -> Option<#device_id> {
                        unsafe {
                            let device = #path :: get_instance_raw();
                            #device_id::new(&UNIQUE, device, #(#get_args),*)
                        }
                    }
                }
            }
        }
    }
}

/// Information about where to get constructor properties for arguments.
///
/// At this point, we assume these all come from the current node.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", content = "value")]
pub enum ArgInfo {
    /// The arguments come from a 'reg' property.
    Reg,
    /// A count of the number of child nodes.
    ChildCount,
}

impl ArgInfo {
    /// Extra properties for the argument, assembling the arguents that should be passed in.
    fn args(&self, node: &Node) -> TokenStream {
        match self {
            ArgInfo::Reg => {
                let reg = node.get_numbers("reg").unwrap();
                quote! {
                    #(#reg),*
                }
            }
            ArgInfo::ChildCount => {
                let count = node.children.len();
                quote! {
                    #count
                }
            }
        }
    }
}

/// Split a path given by a user into a token stream.
fn str_to_path(path: &str) -> TokenStream {
    let names = path.split("::").map(|n| format_ident!("{}", n));
    quote! {
        #(#names)::*
    }
}

/// Load a file of the given name.
pub fn load_augments<P: AsRef<Path>>(name: P) -> Result<Vec<Augmentation>> {
    let fd = File::open(name)?;
    let augs: Vec<Augmentation> = serde_yaml_ng::from_reader(fd)?;
    Ok(augs)
}
