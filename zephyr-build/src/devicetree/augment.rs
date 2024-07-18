//! Support for augmenting the device tree.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::devicetree::output::dt_to_lower_id;

use super::{DeviceTree, Node, Word};

/// This action is given to each node in the device tree, and it is given a chance to return
/// additional code to be included in the module associated with that entry.  These are all
/// assembled together and included in the final generated devicetree.rs.
pub trait Augment {
    /// The default implementation checks if this node matches and calls a generator if it does, or
    /// does nothing if not.
    fn augment(&self, node: &Node, tree: &DeviceTree) -> TokenStream {
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

/// Return all of the Augments we wish to apply.
pub fn get_augments() -> Vec<Box<dyn Augment>> {
    vec![
        Box::new(GpioAugment),
        Box::new(GpioLedsAugment),
        Box::new(FlashControllerAugment),
        Box::new(FlashPartitionAugment),
        Box::new(SinglePartitionAugment),
        Box::new(LabelAugment),
    ]
}

/// Augment gpio nodes.  Gpio nodes are indicated by the 'gpio-controller' property, and not a
/// particular compatible.
struct GpioAugment;

impl Augment for GpioAugment {
    fn is_compatible(&self, node: &Node) -> bool {
        node.has_prop("gpio-controller")
    }

    fn generate(&self, node: &Node, _tree: &DeviceTree) -> TokenStream {
        let ord = node.ord;
        let device = format_ident!("__device_dts_ord_{}", ord);
        quote! {
            pub unsafe fn get_instance_raw() -> *const crate::raw::device {
                &crate::raw::#device
            }
            pub fn get_instance() -> crate::sys::gpio::Gpio {
                let device = unsafe { get_instance_raw() };
                crate::sys::gpio::Gpio {
                    device,
                }
            }
        }
    }
}

/// Augment the individual led nodes.  This provides a safe wrapper that can be used to directly use
/// these nodes.
struct GpioLedsAugment;

impl Augment for GpioLedsAugment {
    // GPIO Leds are nodes whose parent is compatible with "gpio-leds".
    fn is_compatible(&self, node: &Node) -> bool {
        if let Some(parent) = node.parent.borrow().as_ref() {
            parent.is_compatible("gpio-leds")
        } else {
            false
        }
    }

    fn generate(&self, node: &Node, _tree: &DeviceTree) -> TokenStream {
        // TODO: Generalize this.
        let words = node.get_words("gpios").unwrap();
        let target = if let Word::Phandle(gpio) = &words[0] {
            gpio.node_ref()
        } else {
            panic!("gpios property in node is empty");
        };

        // At this point, we support gpio-cells of 2.
        if target.get_number("#gpio-cells") != Some(2) {
            panic!("gpios only support with #gpio-cells of 2");
        }

        if words.len() != 3 {
            panic!("gpio-leds, gpios property expected to have only one entry");
        }

        let pin = words[1].get_number().unwrap();
        let flags = words[2].get_number().unwrap();

        let gpio_route = target.route_to_rust();
        quote! {
            pub fn get_instance() -> crate::sys::gpio::GpioPin {
                unsafe {
                    let port = #gpio_route :: get_instance_raw();
                    crate::sys::gpio::GpioPin {
                        pin: crate::raw::gpio_dt_spec {
                            port,
                            pin: #pin as crate::raw::gpio_pin_t,
                            dt_flags: #flags as crate::raw::gpio_dt_flags_t,
                        }
                    }
                }
            }
        }
    }
}

/// Augment flash controllers.
///
/// Flash controllers are a little weird, because there is no unified compatible that says they
/// implement the flash interface.  In fact, they seem to generally not be accessed through the
/// device tree at all.
struct FlashControllerAugment;

impl Augment for FlashControllerAugment {
    // For now, just look for specific ones we know.
    fn is_compatible(&self, node: &Node) -> bool {
        node.is_compatible("nordic,nrf52-flash-controller") ||
            node.is_compatible("raspberrypi,pico-flash-controller")
    }

    fn generate(&self, node: &Node, _tree: &DeviceTree) -> TokenStream {
        // For now, just return the device node.
        let ord = node.ord;
        let device = format_ident!("__device_dts_ord_{}", ord);
        quote! {
            pub unsafe fn get_instance_raw() -> *const crate::raw::device {
                &crate::raw::#device
            }

            pub fn get_instance() -> crate::sys::flash::FlashController {
                let device = unsafe { get_instance_raw() };
                crate::sys::flash::FlashController {
                    device,
                }
            }
        }
    }
}

/// Augment flash partitions.
///
/// This provides access to the individual partitions via named modules under the flash device.
struct FlashPartitionAugment;

impl Augment for FlashPartitionAugment {
    fn is_compatible(&self, node: &Node) -> bool {
        node.compatible_path(&[Some("fixed-partitions"), Some("soc-nv-flash")])
    }

    fn generate(&self, node: &Node, _tree: &DeviceTree) -> TokenStream {
        let children = node.children.iter().map(|child| {
            let label = child.get_single_string("label").unwrap();
            let label = dt_to_lower_id(label);
            let reg = child.get_numbers("reg").unwrap();
            if reg.len() != 2 {
                panic!("flash partition entry must have 2 reg values");
            }
            let offset = reg[0];
            let size = reg[1];

            quote! {
                pub mod #label {
                    pub fn get_instance() -> crate::sys::flash::FlashPartition {
                        let controller = super::super::super::get_instance();
                        crate::sys::flash::FlashPartition {
                            controller,
                            offset: #offset,
                            size: #size,
                        }
                    }
                }
            }
        });
        quote! {
            #(#children)*
        }
    }
}

/// Augment the partitions themselves rather than the whole partition table.
struct SinglePartitionAugment;

impl Augment for SinglePartitionAugment {
    fn is_compatible(&self, node: &Node) -> bool {
        node.compatible_path(&[None, Some("fixed-partitions"), Some("soc-nv-flash")])
    }

    fn generate(&self, node: &Node, _tree: &DeviceTree) -> TokenStream {
        let reg = node.get_numbers("reg").unwrap();
        if reg.len() != 2 {
            panic!("flash partition entry must have 2 reg values");
        }
        let offset = reg[0];
        let size = reg[1];

        quote! {
            pub fn get_instance() -> crate::sys::flash::FlashPartition {
                let controller = super::super::super::get_instance();
                crate::sys::flash::FlashPartition {
                    controller,
                    offset: #offset,
                    size: #size,
                }
            }
        }
    }
}

/// Add in all of the labels.
struct LabelAugment;

impl Augment for LabelAugment {
    fn is_compatible(&self, node: &Node) -> bool {
        // Insert the labels at the root node.
        println!("Augment check: {}", node.path);
        if let Some(parent) = node.parent.borrow().as_ref() {
            println!("  parent: {}", parent.path);
        }
        node.parent.borrow().is_none()
    }

    fn generate(&self, _node: &Node, tree: &DeviceTree) -> TokenStream {
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
                #(#nodes)*
            }
        }
    }
}
