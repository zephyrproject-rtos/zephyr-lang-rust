//! Versioned interchange with Zephyr's edtlib. References use absolute node paths,
//! so unlabeled nodes can be referenced without inventing public Rust labels.
use super::*;
use anyhow::{bail, Context};
use serde::Deserialize;

#[derive(Deserialize)]
struct Export {
    version: u32,
    root: Entry,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    path: String,
    ord: usize,
    labels: Vec<String>,
    properties: Vec<Property>,
    children: Vec<Entry>,
}

impl DeviceTree {
    pub fn from_edt(path: &Path) -> anyhow::Result<Self> {
        let data: Export = serde_json::from_str(&std::fs::read_to_string(path)?)
            .with_context(|| format!("reading EDT export {}", path.display()))?;
        if data.version != 1 {
            bail!("unsupported EDT export version {}", data.version);
        }
        let mut labels = BTreeMap::new();
        let mut paths = BTreeMap::new();
        fn build(
            entry: Entry,
            route: Vec<String>,
            labels: &mut BTreeMap<String, Rc<Node>>,
            paths: &mut BTreeMap<String, Rc<Node>>,
        ) -> Rc<Node> {
            let children = entry
                .children
                .into_iter()
                .map(|child| {
                    let mut route = route.clone();
                    route.push(child.name.clone());
                    build(child, route, labels, paths)
                })
                .collect();
            let node = Rc::new(Node {
                name: entry.name,
                path: entry.path,
                route,
                ord: entry.ord,
                labels: entry.labels,
                properties: entry.properties,
                children,
                parent: RefCell::new(None),
            });
            for label in &node.labels {
                labels.insert(label.clone(), node.clone());
            }
            paths.insert(node.path.clone(), node.clone());
            node
        }
        let root = build(data.root, vec![], &mut labels, &mut paths);
        // Resolve against paths without exposing these internal keys as labels.
        root.phandle_walk(&paths);
        root.parent_walk();
        Ok(Self { root, labels })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_ordinals_labels_and_unlabeled_references() {
        let path = std::env::temp_dir().join(format!("edt-test-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"version":1,"root":{"name":"/","path":"/","ord":0,"labels":[],"properties":[],"children":[{"name":"gpio@0","path":"/gpio@0","ord":7,"labels":[],"properties":[],"children":[]},{"name":"led","path":"/led","ord":9,"labels":["led0"],"properties":[{"name":"gpios","value":[{"Words":[{"Phandle":"/gpio@0"},{"Number":3}]}]}],"children":[]}]}}"#).unwrap();
        let tree = DeviceTree::from_edt(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let led = &tree.labels["led0"];
        assert_eq!(led.ord, 9);
        assert_eq!(tree.labels.len(), 1);
        let words = led.get_words("gpios").unwrap();
        let Word::Phandle(reference) = &words[0] else {
            panic!("missing reference")
        };
        assert_eq!(reference.node_ref().ord, 7);
        assert!(Rc::ptr_eq(
            led.parent.borrow().as_ref().unwrap(),
            &tree.root
        ));
        assert!(tree.to_tokens(&[]).to_string().contains("gpio_0"));
    }
}
