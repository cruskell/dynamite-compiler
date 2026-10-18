//! Preserve external function identities while making GNU inline bodies callable.

use super::*;

fn visit_direct_calls(node: &mut Node, operation: &mut impl FnMut(&mut String)) {
    if node.nt == NodeType::CallFunc && node.lhs.is_none() {
        operation(&mut node.global_name);
    }
    for child in node.child_nodes_mut() {
        visit_direct_calls(child, operation);
    }
}

impl AstBuilderImpl<'_> {
    pub(super) fn lower_gnu_inline_calls(&mut self) {
        let candidates: HashSet<_> = self
            .functions
            .iter()
            .filter(|(_, function)| {
                function.gnu_inline && function.inline_only() && function.body.is_some()
            })
            .map(|(name, _)| name.clone())
            .collect();
        if candidates.is_empty() {
            return;
        }
        let mut referenced = HashSet::new();
        let mut pending = vec![];
        for function in self
            .functions
            .values_mut()
            .filter(|function| !function.inline_only())
        {
            if let Some(body) = &mut function.body {
                visit_direct_calls(body, &mut |name| {
                    if candidates.contains(name) && referenced.insert(name.clone()) {
                        pending.push(name.clone());
                    }
                });
            }
        }
        // Only materialize reachable bodies: unused header helpers may call undefined symbols.
        while let Some(name) = pending.pop() {
            let body = self
                .functions
                .get_mut(&name)
                .unwrap()
                .body
                .as_mut()
                .unwrap();
            visit_direct_calls(body, &mut |callee| {
                if candidates.contains(callee) && referenced.insert(callee.clone()) {
                    pending.push(callee.clone());
                }
            });
        }
        let mut names = HashMap::new();
        for name in referenced {
            let local = format!(".inline.{name}");
            let function = self.functions.get_mut(&name).unwrap();
            let local_function = {
                let body = function.body.take();
                let args = std::mem::take(&mut function.args);
                Func {
                    token: function.token.clone(),
                    cty: function.cty.clone(),
                    args,
                    body,
                    offset_size: function.offset_size,
                    return_pointer: function.return_pointer,
                    internal: true,
                    external_definition: true,
                    noinline: function.noinline,
                    ..Func::default()
                }
            };
            self.functions.insert(local.clone(), local_function);
            names.insert(name, local);
        }
        for function in self
            .functions
            .values_mut()
            .filter(|function| !function.inline_only())
        {
            if let Some(body) = &mut function.body {
                visit_direct_calls(body, &mut |name| {
                    if let Some(local) = names.get(name) {
                        *name = local.clone();
                    }
                });
            }
        }
    }
}
