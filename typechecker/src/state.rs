use std::collections::HashMap;

use crate::{
    ast_visitor::TraversalVisitor,
    diagnostic::Diagnostic,
    nodes::EnderpyFile,
    ruff_python_import_resolver as ruff_python_resolver,
    ruff_python_import_resolver::{
        import_result::ImportResult, module_descriptor::ImportModuleDescriptor, resolver,
    },
    semantic_analyzer::SemanticAnalyzer,
    symbol_table::SymbolTable,
};

#[derive(Debug, Clone)]
pub struct State {
    pub file: EnderpyFile,
    symbol_table: SymbolTable,
    pub diagnostics: Vec<Diagnostic>,
    // Map of import names to the result of the import
    pub imports: HashMap<String, ImportResult>,
}

impl State {
    pub fn new(file: EnderpyFile) -> Self {
        Self {
            file,
            symbol_table: SymbolTable::global(),
            diagnostics: Vec::new(),
            imports: HashMap::new(),
        }
    }
    /// entry point to fill up the symbol table from the global definitions
    pub fn populate_symbol_table(&mut self) {
        let mut sem_anal = SemanticAnalyzer::new(self.file.clone(), self.imports.clone());
        for stmt in &self.file.body {
            sem_anal.visit_stmt(stmt)
        }
        self.symbol_table = sem_anal.globals
    }

    pub fn get_symbol_table(&self) -> SymbolTable {
        self.symbol_table.clone()
    }

    pub fn resolve_file_imports(
        &mut self,
        execution_environment: &ruff_python_resolver::execution_environment::ExecutionEnvironment,
        import_config: &ruff_python_resolver::config::Config,
        host: &ruff_python_resolver::host::StaticHost,
    ) {
        log::debug!("resolving imports for file: {}", self.file.module_name());
        for import in self.file.imports.iter() {
            let import_descriptions = match import {
                crate::nodes::ImportKinds::Import(i) => i
                    .names
                    .iter()
                    .map(|x| {
                        ruff_python_resolver::module_descriptor::ImportModuleDescriptor::from(x)
                    })
                    .collect::<Vec<ImportModuleDescriptor>>(),
                crate::nodes::ImportKinds::ImportFrom(i) => {
                    vec![ruff_python_resolver::module_descriptor::ImportModuleDescriptor::from(i)]
                }
            };
            log::debug!("import descriptions: {:?}", import_descriptions);

            for import_desc in import_descriptions {
                let resolved = resolver::resolve_import(
                    self.file.path().as_path(),
                    execution_environment,
                    &import_desc,
                    import_config,
                    host,
                );
                if !resolved.is_import_found {
                    let error = format!("cannot import name '{}'", import_desc.name());
                    log::warn!("{}", error);
                    continue;
                }
                log::debug!(
                    "resolved import: {} -> {:?}",
                    import_desc.name(),
                    resolved.resolved_paths
                );
                self.imports.insert(import_desc.name(), resolved);
            }
        }
    }
}
