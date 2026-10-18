use super::{Node, NodeType};

impl Node {
    /// Direct children in structural order, not source-language evaluation order.
    pub(crate) fn child_nodes(&self) -> impl Iterator<Item = &Self> {
        [
            &self.lhs, &self.rhs, &self.cond, &self.then, &self.els, &self.ini, &self.upd,
        ]
        .into_iter()
        .filter_map(|child| child.as_deref())
        .chain(&self.children)
        .chain(&self.args)
    }

    pub(crate) fn child_nodes_mut(&mut self) -> impl Iterator<Item = &mut Self> {
        [
            &mut self.lhs,
            &mut self.rhs,
            &mut self.cond,
            &mut self.then,
            &mut self.els,
            &mut self.ini,
            &mut self.upd,
        ]
        .into_iter()
        .filter_map(|child| child.as_deref_mut())
        .chain(&mut self.children)
        .chain(&mut self.args)
    }

    pub fn contains_case(&self) -> bool {
        if matches!(self.nt, NodeType::Switch) {
            return false;
        }
        if matches!(self.nt, NodeType::Case | NodeType::Label) {
            return true;
        }
        self.children.iter().any(Self::contains_case)
            || [
                &self.lhs, &self.rhs, &self.cond, &self.then, &self.els, &self.ini, &self.upd,
            ]
            .into_iter()
            .flatten()
            .any(|node| node.contains_case())
    }
}
