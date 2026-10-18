use super::AsmGenerator;
use crate::generator::aarch64::assembly::Assembly;
use crate::generator::globals;
use crate::global::GlobalVariable;
use crate::token::Token;

impl AsmGenerator<'_> {
    pub(super) fn gen_string_literals(&self, literals: &[Token]) -> Assembly {
        globals::gen_string_literals(self.target_os, literals)
    }

    pub(super) fn weak_reference(&self, name: &str) -> Assembly {
        globals::weak_reference(self.target_os, name, &|name| self.with_prefix(name))
    }

    pub(super) fn gen_global_variable(&self, name: &str, variable: &GlobalVariable) -> Assembly {
        globals::gen_global_variable(
            self.target_os,
            name,
            variable,
            &|name| self.with_prefix(name),
            &|name| self.address_symbol(name),
        )
    }
}
