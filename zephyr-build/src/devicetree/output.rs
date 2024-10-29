//! Outputting the devicetree into Rust.

// We output the device tree in a module tree in Rust that mirrors the DTS tree.  Devicetree names
// are made into valid Rust identifiers by the simple rule that invalid characters are replaced with
// underscores.
//
// The actual output is somewhat specialized, and driven by the data, and the compatible values.
// Support for particular devices should also be added to the device tree here, so that the nodes
// make sense for that device, and that there are general accessors that return wrapped node types.

use std::io::Write;

use anyhow::Result;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use super::{augment::Augment, DeviceTree, Node, Property, Value, Word};

impl DeviceTree {
    /// Generate a TokenStream for the Rust representation of this device tree.
    pub fn to_tokens(&self, augments: &[Box<dyn Augment>]) -> TokenStream {

        // Root is a little special.  Since we don't want a module for this (it will be provided
        // above where it is included, so it can get documentation and attributes), we use None for
        // the name.
        self.node_walk(self.root.as_ref(), None, &augments)
    }

    // Write, to the given writer, CFG lines so that Rust code can conditionalize based on the DT.
    pub fn output_node_paths<W: Write>(&self, write: &mut W) -> Result<()> {
        self.root.as_ref().output_path_walk(write, None)?;

        // Also, output all of the labels.  Technically, this depends on the labels augment being
        // present.
        writeln!(write, "cargo:rustc-cfg=dt=\"labels\"")?;
        for label in self.labels.keys() {
            writeln!(write, "cargo:rustc-cfg=dt=\"labels::{}\"", fix_id(label))?;
        }
        Ok(())
    }

    fn node_walk(&self, node: &Node, name: Option<&str>, augments: &[Box<dyn Augment>]) -> TokenStream {
        let children = node.children.iter().map(|child| {
            self.node_walk(child.as_ref(), Some(&child.name), augments)
        });
        // Simplistic first pass, turn the properties into constents of the formatted text of the
        // property.
        let props = node.properties.iter().map(|prop| {
            self.property_walk(prop)
        });
        let ord = node.ord;

        // Open the parent as a submodule.  This is the same as 'super', so not particularly useful.
        /*
        let parent = if let Some(parent) = node.parent.borrow().as_ref() {
            let route = parent.route_to_rust();
            quote! {
                pub mod silly_super {
                    pub use #route::*;
                }
            }
        } else {
            TokenStream::new()
        };
        */

        // If this is compatible with an augment, use the augment to add any additional properties.
        let augs = augments.iter().map(|aug| aug.augment(node, self));

        if let Some(name) = name {
            let name_id = dt_to_lower_id(name);
            quote! {
                pub mod #name_id {
                    pub const ORD: usize = #ord;
                    #(#props)*
                    #(#children)*
                    // #parent
                    #(#augs)*
                }
            }
        } else {
            quote! {
                #(#props)*
                #(#children)*
                #(#augs)*
            }
        }
    }

    // This is the "fun" part.  We try to find some patterns that can be formatted more nicely, but
    // otherwise they are just somewhat simply converted.
    fn property_walk(&self, prop: &Property) -> TokenStream {
        // Pattern matching is rather messy at this point.
        if let Some(value) = prop.get_single_value() {
            match value {
                Value::Words(ref words) => {
                    if words.len() == 1 {
                        match &words[0] {
                            Word::Number(n) => {
                                let tag = dt_to_upper_id(&prop.name);
                                return quote! {
                                    pub const #tag: u32 = #n;
                                };
                            }
                            _ => return general_property(prop),
                        }
                    } else {
                        return general_property(prop);
                    }
                }
                Value::Phandle(ref ph) => {
                    let target = ph.node_ref();
                    let route = target.route_to_rust();
                    let tag = dt_to_lower_id(&prop.name);
                    return quote! {
                        pub mod #tag {
                            pub use #route::*;
                        }
                    }
                }
                _ => return general_property(prop),
            }
        }
        general_property(prop)
    }
}

impl Node {
    /// Return the route to this node, as a Rust token stream giving a fully resolved name of the
    /// route.
    pub fn route_to_rust(&self) -> TokenStream {
        let route: Vec<_> = self.route.iter().map(|p| dt_to_lower_id(p)).collect();
        quote! {
            crate :: devicetree #(:: #route)*
        }
    }

    /// Walk this tree of nodes, writing out the path names of the nodes that are present.  The name
    /// of None, indicates the root node.
    fn output_path_walk<W: Write>(&self, write: &mut W, name: Option<&str>) -> Result<()> {
        for child in &self.children {
            let fixed_name = fix_id(&child.name);
            let child_name = if let Some(name) = name {
                format!("{}::{}", name, fixed_name)
            } else {
                fixed_name
            };

            writeln!(write, "cargo:rustc-cfg=dt=\"{}\"", child_name)?;

            for prop in &child.properties {
                prop.output_path(write, &child_name)?;
            }

            child.output_path_walk(write, Some(&child_name))?;
        }

        Ok(())
    }
}

impl Property {
    // Return property values that consist of a single value.
    fn get_single_value(&self) -> Option<&Value> {
        if self.value.len() == 1 {
            Some(&self.value[0])
        } else {
            None
        }
    }

    // If this property is a single top-level phandle, output that a that path is valid.  It isn't a
    // real node, but acts like one.
    fn output_path<W: Write>(&self, write: &mut W, name: &str) -> Result<()> {
        if let Some(value) = self.get_single_value() {
            if let Value::Phandle(_) = value {
                writeln!(write, "cargo:rustc-cfg=dt=\"{}::{}\"", name, fix_id(&self.name))?;
            }
        }
        Ok(())
    }
}

fn general_property(prop: &Property) -> TokenStream {
    let text = format!("{:?}", prop.value);
    let tag = format!("{}_DEBUG", prop.name);
    let tag = dt_to_upper_id(&tag);
    quote! {
        pub const #tag: &'static str = #text;
    }
}

/// Given a DT name, return an identifier for a lower-case version.
pub fn dt_to_lower_id(text: &str) -> Ident {
    format_ident!("{}", fix_id(&text))
}

pub fn dt_to_upper_id(text: &str) -> Ident {
    format_ident!("{}", fix_id(&text.to_uppercase()))
}

/// Fix a devicetree identifier to be safe as a rust identifier.
fn fix_id(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        match ch {
            '#' => result.push('N'),
            '-' => result.push('_'),
            '@' => result.push('_'),
            ',' => result.push('_'),
            ch => result.push(ch),
        }
    }
    result
}
