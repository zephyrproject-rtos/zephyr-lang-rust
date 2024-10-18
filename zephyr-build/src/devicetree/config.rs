//! Config handling.
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

use crate::devicetree::{output::dt_to_lower_id, Word};

use super::{augment::Augment, DeviceTree, Node};

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
    },
    /// Generate all of the labels as its own node.
    Labels,
}

impl Action {
    fn generate(&self, _name: &Ident, node: &Node, tree: &DeviceTree) -> TokenStream {
        match self {
            Action::Instance { raw, device } => {
                raw.generate(node, device)
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", content = "value")]
pub enum RawInfo {
    /// Get the raw device directly from this node.
    Myself,
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
    fn generate(&self, node: &Node, device: &str) -> TokenStream {
        let device_id = str_to_path(device);
        match self {
            RawInfo::Myself => {
                let ord = node.ord;
                let rawdev = format_ident!("__device_dts_ord_{}", ord);
                quote! {
                    /// Get the raw `const struct device *` of the device tree generated node.
                    pub unsafe fn get_instance_raw() -> *const crate::raw::device {
                        &crate::raw::#rawdev
                    }
                    pub fn get_instance() -> #device_id {
                        unsafe {
                            let device = get_instance_raw();
                            #device_id::new(device)
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
                    pub fn get_instance() -> #device_id {
                        unsafe {
                            let device = #target_route :: get_instance_raw();
                            #device_id::new(device, #(#args),*)
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
                    pub fn get_instance() -> #device_id {
                        unsafe {
                            let device = #path :: get_instance_raw();
                            #device_id::new(device, #(#get_args),*)
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
pub fn load<P: AsRef<Path>>(name: P) -> Result<Vec<Augmentation>> {
    let fd = File::open(name)?;
    let augs: Vec<Augmentation> = serde_yaml_ng::from_reader(fd)?;
    Ok(augs)
}

/// Output a sample yaml file, so we can understand the format.
pub fn sample() {
    use std::fs::File;

    let data = vec![
        Augmentation {
            name: "gpio-controller".to_string(),
            rules: vec![
                Rule::HasProp("gpio-controller".to_string()),
            ],
            actions: vec![
                Action::Instance {
                    raw: RawInfo::Myself,
                    device: "crate::sys::gpio::Gpio".to_string(),
                },
            ],
        },
        Augmentation {
            name: "gpio-leds".to_string(),
            rules: vec![
                Rule::Compatible {
                    names: vec!["gpio-leds".to_string()],
                    level: 1,
                },
            ],
            actions: vec![
                Action::Instance {
                    raw: RawInfo::Phandle("gpios".to_string()),
                    device: "crate::sys::gpio::GpioPin".to_string(),
                }
            ],
        },
    ];
    let fd = File::create("dt-sample.yaml").unwrap();
    serde_yaml_ng::to_writer(fd, &data).unwrap();
}
