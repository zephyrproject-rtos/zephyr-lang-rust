//! Incorporating Zephyr's devicetree into Rust.
//!
//! Zephyr depends fairly heavily on the devicetree for configuration.  The build system reads
//! multiple DTS files, and coalesces this into a single devicetree.  This tree is output in a few
//! different ways:
//!
//! - Canonical DTS.  There is a single DTS file (`build/zephyr/zephyr.dts`) that contains the final
//! tree, but still in DTS format (the DTB file would have information discarded).
//!
//! - Generated.  The C header `devicetree_generated.h` contains all of the definitions.  This isn't
//! a particularly friendly file to read or parse, but it does have one piece of information that is
//! not represented anywhere else: the mapping between devicetree nodes and their "ORD" index.  The
//! device nodes in the system are indexed by this number, and we need this in order to be able to
//! reference the nodes from Rust.
//!
//! Beyond the ORD field, it seems easier to deal with the DTS file itself.  Parsing is fairly
//! straightforward, as it is a subset of the DTS format, and we only have to be able to deal with
//! the files that are generated by the Zephyr build process.

// TODO: Turn this off.
#![allow(dead_code)]

use ordmap::OrdMap;
use std::{cell::RefCell, collections::BTreeMap, path::Path, rc::Rc};

mod augment;
mod ordmap;
mod output;
mod parse;

pub use augment::{Augment, load_augments};

pub struct DeviceTree {
    /// The root of the tree.
    root: Rc<Node>,
    /// All of the labels.
    labels: BTreeMap<String, Rc<Node>>,
}

// This is a single node in the devicetree.
pub struct Node {
    // The name of the node itself.
    name: String,
    // The full path of this node in the tree.
    path: String,
    // The "route" is the path, but still as separate entries.
    route: Vec<String>,
    // The ord index in this particular Zephyr build.
    ord: usize,
    // Labels attached to this node.
    labels: Vec<String>,
    // Any properties set in this node.
    properties: Vec<Property>,
    // Children nodes.
    children: Vec<Rc<Node>>,
    // The parent. Should be non-null except at the root node.
    parent: RefCell<Option<Rc<Node>>>,
}

#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub value: Vec<Value>,
}

// Although the real device flattends all of these into bytes, Zephyr takes advantage of them at a
// slightly higher level.
#[derive(Debug)]
pub enum Value {
    Words(Vec<Word>),
    Bytes(Vec<u8>),
    Phandle(Phandle), // TODO
    String(String),
}

/// A phandle is a named reference to a labeled part of the DT.  We resolve this by making the
/// reference optional, and filling them in afterwards.
pub struct Phandle {
    /// The label of our target.  Keep this because it may be useful to know which label was used,
    /// as nodes often have multiple labels.
    name: String,
    /// The inside of the node, inner mutability so this can be looked up and cached.
    node: RefCell<Option<Rc<Node>>>,
}

#[derive(Debug)]
pub enum Word {
    Number(u32),
    Phandle(Phandle),
}

impl DeviceTree {
    /// Decode the zephyr.dts and devicetree_generated.h files from the build and build an internal
    /// representation of the devicetree itself.
    pub fn new<P1: AsRef<Path>, P2: AsRef<Path>>(dts_path: P1, dt_gen: P2) -> DeviceTree {
        let ords = OrdMap::new(dt_gen);

        let dts = std::fs::read_to_string(dts_path)
            .expect("Reading zephyr.dts file");
        let dt = parse::parse(&dts, &ords);
        dt.resolve_phandles();
        dt.set_parents();
        dt
    }

    /// Walk the node tree, fixing any phandles to include their reference.
    fn resolve_phandles(&self) {
        self.root.phandle_walk(&self.labels);
    }

    /// Walk the node tree, setting each node's parent appropriately.
    fn set_parents(&self) {
        self.root.parent_walk();
    }
}

impl Node {
    fn phandle_walk(&self, labels: &BTreeMap<String, Rc<Node>>) {
        for prop in &self.properties {
            for value in &prop.value {
                value.phandle_walk(labels);
            }
        }
        for child in &self.children {
            child.phandle_walk(labels);
        }
    }

    fn parent_walk(self: &Rc<Self>) {
        // *(self.parent.borrow_mut()) = Some(parent.clone());

        for child in &self.children {
            *(child.parent.borrow_mut()) = Some(self.clone());
            child.parent_walk()
        }
    }

    fn is_compatible(&self, name: &str) -> bool {
        if let Some(prop) = self.properties.iter().find(|p| p.name == "compatible") {
            prop.value.iter().any(|v| {
                match v {
                    Value::String(vn) if name == vn => true,
                    _ => false,
                }
            })
        } else {
            // If there is no compatible field, we are clearly not compatible.
            false
        }
    }

    /// A richer compatible test.  Walks a series of names, in reverse.  Any that are "Some(x)" must
    /// be compatible with "x" at that level.
    fn compatible_path(&self, path: &[Option<&str>]) -> bool {
        let res = self.path_walk(path, 0);
        // println!("compatible? {}: {} {:?}", res, self.path, path);
        res
    }

    /// Recursive path walk, to make borrowing simpler.
    fn path_walk(&self, path: &[Option<&str>], pos: usize) -> bool {
        if pos >= path.len() {
            // Once past the end, we consider everything a match.
            return true;
        }

        // Check the failure condition, where this node isn't compatible with this section of the path.
        if let Some(name) = path[pos] {
            if !self.is_compatible(name) {
                return false;
            }
        }

        // Walk down the tree.  We have to check for None here, as we can't recurse on the none
        // case.
        if let Some(child) = self.parent.borrow().as_ref() {
            child.path_walk(path, pos + 1)
        } else {
            // We've run out of nodes, so this is considered not matching.
            false
        }
    }

    /// Is the named property present?
    fn has_prop(&self, name: &str) -> bool {
        self.properties.iter().any(|p| p.name == name)
    }

    /// Get this property in its entirety.
    fn get_property(&self, name: &str) -> Option<&[Value]> {
        for p in &self.properties {
            if p.name == name {
                return Some(&p.value);
            }
        }
        return None;
    }

    /// Attempt to retrieve the named property, as a single entry of Words.
    fn get_words(&self, name: &str) -> Option<&[Word]> {
        self.get_property(name)
            .and_then(|p| {
                match p {
                    &[Value::Words(ref w)] => Some(w.as_ref()),
                    _ => None,
                }
            })
    }

    /// Get a property that consists of a single number.
    fn get_number(&self, name: &str) -> Option<u32> {
        self.get_words(name)
            .and_then(|p| {
                if let &[Word::Number(n)] = p {
                    Some(n)
                } else {
                    None
                }
            })
    }

    /// Get a property that consists of multiple numbers.
    fn get_numbers(&self, name: &str) -> Option<Vec<u32>> {
        let mut result = vec![];
        for word in self.get_words(name)? {
            if let Word::Number(n) = word {
                result.push(*n);
            } else {
                return None;
            }
        }
        Some(result)
    }

    /// Get a property that is a single string.
    fn get_single_string(&self, name: &str) -> Option<&str> {
        self.get_property(name)
            .and_then(|p| {
                if let &[Value::String(ref text)] = p {
                    Some(text.as_ref())
                } else {
                    None
                }
            })
    }
}

impl Value {
    fn phandle_walk(&self, labels: &BTreeMap<String, Rc<Node>>) {
        match self {
            Value::Phandle(ph) => ph.phandle_resolve(labels),
            Value::Words(words) => {
                for w in words {
                    match w {
                        Word::Phandle(ph) => ph.phandle_resolve(labels),
                        _ => (),
                    }
                }
            }
            _ => (),
        }
    }
}

impl Phandle {
    /// Construct a phandle that is unresolved.
    pub fn new(name: String) -> Phandle {
        Phandle {
            name,
            node: RefCell::new(None),
        }
    }

    /// Resolve this phandle, with the given label for lookup.
    fn phandle_resolve(&self, labels: &BTreeMap<String, Rc<Node>>) {
        // If already resolve, just return.
        if self.node.borrow().is_some() {
            return;
        }

        let node = labels.get(&self.name).cloned()
            .expect("Missing phandle");
        *self.node.borrow_mut() = Some(node);
    }

    /// Get the child node, panicing if it wasn't resolved properly.
    fn node_ref(&self) -> Rc<Node> {
        self.node.borrow().as_ref().unwrap().clone()
    }
}

impl Word {
    pub fn get_number(&self) -> Option<u32> {
        match self {
            Word::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn get_phandle(&self) -> Option<&Phandle> {
        match self {
            Word::Phandle(ph) => Some(ph),
            _ => None,
        }
    }
}

// To avoid recursion, the debug printer for Phandle just prints the name.
impl std::fmt::Debug for Phandle {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "Phandle({:?})", self.name)
    }
}
