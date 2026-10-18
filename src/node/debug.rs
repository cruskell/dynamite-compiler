use super::{Node, NodeType};

impl Node {
    pub fn to_debug_string(&self, indent: usize) -> String {
        match self.nt {
            NodeType::Num => {
                format!("Num: {}", self.value.unwrap())
            }
            NodeType::LocalVar => {
                format!(
                    "LocalVar: {{ type: {:?}, offset: {} }}",
                    self.cty.as_ref().unwrap(),
                    self.offset.unwrap()
                )
            }
            NodeType::GlobalVar => {
                format!("GlobalVar: {{ name: {}{} }}", &self.global_name, &self.dest)
            }
            _ => [
                vec![format!("{:?}:", self.nt)],
                self.ini
                    .iter()
                    .map(|c| {
                        format!(
                            "{}ini->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.cond
                    .iter()
                    .map(|c| {
                        format!(
                            "{}cond->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.upd
                    .iter()
                    .map(|c| {
                        format!(
                            "{}upd->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.then
                    .iter()
                    .map(|c| {
                        format!(
                            "{}then->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.els
                    .iter()
                    .map(|c| {
                        format!(
                            "{}els->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.lhs
                    .iter()
                    .map(|c| {
                        format!(
                            "{}lhs->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.rhs
                    .iter()
                    .map(|c| {
                        format!(
                            "{}rhs->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.children
                    .iter()
                    .map(|c| {
                        format!(
                            "{}children->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
                self.args
                    .iter()
                    .map(|c| {
                        format!(
                            "{}args->{}",
                            " ".repeat(indent),
                            c.to_debug_string(indent + 2)
                        )
                    })
                    .collect::<Vec<String>>(),
            ]
            .concat()
            .join("\n"),
        }
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_debug_string(2))
    }
}
