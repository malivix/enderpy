use std::{collections::HashMap, fmt::Display};

use enderpy_python_parser::ast::{self, Node};

use crate::{ruff_python_import_resolver::import_result::ImportResult, type_check::builtins};

#[derive(Debug, Clone)]
pub struct SymbolTable {
    // Sub tables are scopes inside the current scope
    // after building symbol table is finished this only contains the most outer scope
    scopes: Vec<SymbolTableScope>,
    // When a symbol goes out of scope we save it here to be able to look it up later
    all_scopes: Vec<SymbolTableScope>,

    /// The distance between the current scope and the scope where the symbol
    /// was defined
    _locals: HashMap<ast::Expression, u8>,
}

#[derive(Debug, Clone)]
pub struct SymbolTableScope {
    pub id: usize,
    pub start_pos: usize,
    pub symbol_table_type: SymbolTableType,
    pub name: String,
    symbols: HashMap<String, SymbolTableNode>,
    parent: Option<usize>,
}

fn get_id() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    COUNTER.fetch_add(1, Ordering::SeqCst)
}

impl SymbolTableScope {
    pub fn new(symbol_table_type: SymbolTableType, name: String, start_line_number: usize) -> Self {
        SymbolTableScope {
            id: get_id(),
            symbol_table_type,
            name,
            symbols: HashMap::new(),
            parent: None,
            start_pos: start_line_number,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum SymbolTableType {
    /// BUILTIN scope is used for builtins like len, print, etc.
    BUILTIN,
    Module,
    Class,
    Function,
}

#[derive(Debug, Clone)]
pub struct SymbolTableNode {
    pub name: String,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone)]
pub struct DeclarationPath {
    pub module_name: String,
    pub node: Node,
}

#[derive(Debug, Clone)]
pub enum Declaration {
    Variable(Variable),
    Function(Function),
    Class(Class),

    // Alias is used for imports
    Alias(Alias),

    Parameter(Paramter),
    // TypeParameterDeclaration represents a type parameter in a generic class or function.
    // It models type parameters declared on classes and functions like T in List[T].
    TypeParameter(TypeParameter),

    TypeAlias(TypeAlias),
}

impl Declaration {
    pub fn declaration_path(&self) -> &DeclarationPath {
        match self {
            Declaration::Variable(v) => &v.declaration_path,
            Declaration::Function(f) => &f.declaration_path,
            Declaration::Class(c) => &c.declaration_path,
            Declaration::Parameter(p) => &p.declaration_path,
            Declaration::Alias(a) => &a.declaration_path,
            Declaration::TypeParameter(t) => &t.declaration_path,
            Declaration::TypeAlias(t) => &t.declaration_path,
        }
    }
}

impl Display for DeclarationPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{:?}", self.module_name, self.node)
    }
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub declaration_path: DeclarationPath,
    pub scope: SymbolScope,
    pub type_annotation: Option<ast::Expression>,
    pub inferred_type_source: Option<ast::Expression>,
    pub is_constant: bool,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub declaration_path: DeclarationPath,
    pub function_node: ast::FunctionDef,
    pub is_method: bool,
    pub is_generator: bool,
    /// return statements that are reachable in the top level function body
    pub return_statements: Vec<ast::Return>,
    /// yield statements that are reachable in the top level function body
    pub yeild_statements: Vec<ast::Yield>,
    /// raise statements that are reachable in the top level function body
    pub raise_statements: Vec<ast::Raise>,
}

impl Function {
    pub fn is_abstract(&self) -> bool {
        if !self.is_method {
            return false;
        }
        for decorator in self.function_node.decorator_list.iter() {
            if let ast::Expression::Name(n) = &decorator {
                if &n.id == "abstractmethod" {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Debug, Clone)]
pub struct Class {
    pub name: String,
    pub declaration_path: DeclarationPath,
    // Method names, can be used to look up the function in the symbol table
    // of the class
    pub methods: Vec<String>,
    // instance attibutes that are defined in the __init__ method
    // if the attribute is referencing another symbol we need to look up that symbol in the
    // __init__ method
    pub attributes: HashMap<String, ast::Expression>,
}

#[derive(Debug, Clone)]
pub struct Paramter {
    pub declaration_path: DeclarationPath,
    pub parameter_node: ast::Arg,
    pub type_annotation: Option<ast::Expression>,
    pub default_value: Option<ast::Expression>,
}

#[derive(Debug, Clone)]
pub struct TypeParameter {
    pub declaration_path: DeclarationPath,
    pub type_parameter_node: ast::TypeParam,
}

#[derive(Debug, Clone)]
pub struct Alias {
    pub declaration_path: DeclarationPath,
    /// The import node that this alias is for. Only one of import_node or
    /// import_from_node will be set
    pub import_from_node: Option<ast::ImportFrom>,
    pub import_node: Option<ast::Import>,
    /// Name of the imported symbol in case of ImportFrom
    /// e.g. From bar import baz -> baz is the symbol name
    pub symbol_name: Option<String>,
    /// The result of the import
    pub import_result: ImportResult,
}

#[derive(Debug, Clone)]
pub struct TypeAlias {
    pub declaration_path: DeclarationPath,
    pub type_alias_node: ast::TypeAlias,
}

pub struct LookupSymbolRequest {
    pub name: String,
    pub position: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub enum SymbolScope {
    Global,
    Nonlocal,
    Local,
    Unknown,
}

impl SymbolTable {
    pub fn global() -> Self {
        let mut builtin_scope = SymbolTableScope {
            id: get_id(),
            symbol_table_type: SymbolTableType::BUILTIN,
            symbols: HashMap::new(),
            name: String::from("builtins"),
            parent: None,
            start_pos: 0,
        };
        // TODO: This will be removed once we can import the builtins from the stdlib
        // Hacky way of putting the builtin in symbol table so I can implement some
        // tests
        let list_class = Class {
            name: builtins::LIST_TYPE.to_string(),
            declaration_path: DeclarationPath {
                module_name: String::from("builtins"),
                node: Node { start: 0, end: 0 },
            },
            methods: vec![],
            attributes: HashMap::new(),
        };
        builtin_scope.symbols.insert(
            builtins::LIST_TYPE.to_string(),
            SymbolTableNode {
                name: builtins::LIST_TYPE.to_string(),
                declarations: vec![Declaration::Class(list_class)],
            },
        );
        let tuple_class = Class {
            name: builtins::TUPLE_TYPE.to_string(),
            declaration_path: DeclarationPath {
                module_name: String::from("builtins"),
                node: Node { start: 0, end: 0 },
            },
            methods: vec![],
            attributes: HashMap::new(),
        };
        builtin_scope.symbols.insert(
            builtins::TUPLE_TYPE.to_string(),
            SymbolTableNode {
                name: builtins::TUPLE_TYPE.to_string(),
                declarations: vec![Declaration::Class(tuple_class)],
            },
        );
        let set_class = Class {
            name: builtins::SET_TYPE.to_string(),
            declaration_path: DeclarationPath {
                module_name: String::from("builtins"),
                node: Node { start: 0, end: 0 },
            },
            methods: vec![],
            attributes: HashMap::new(),
        };
        builtin_scope.symbols.insert(
            builtins::SET_TYPE.to_string(),
            SymbolTableNode {
                name: builtins::SET_TYPE.to_string(),
                declarations: vec![Declaration::Class(set_class)],
            },
        );
        let dict_class = Class {
            name: builtins::DICT_TYPE.to_string(),
            declaration_path: DeclarationPath {
                module_name: String::from("builtins"),
                node: Node { start: 0, end: 0 },
            },
            methods: vec![],
            attributes: HashMap::new(),
        };
        builtin_scope.symbols.insert(
            builtins::DICT_TYPE.to_string(),
            SymbolTableNode {
                name: builtins::DICT_TYPE.to_string(),
                declarations: vec![Declaration::Class(dict_class)],
            },
        );
        let global_scope = SymbolTableScope {
            id: get_id(),
            symbol_table_type: SymbolTableType::Module,
            symbols: HashMap::new(),
            name: String::from("global"),
            parent: Some(builtin_scope.id),
            start_pos: 0,
        };
        SymbolTable {
            scopes: vec![builtin_scope, global_scope],
            all_scopes: vec![],
            _locals: HashMap::new(),
        }
    }

    /// Do not use for lookup operations
    fn current_scope(&self) -> &SymbolTableScope {
        if let Some(scope) = self.scopes.last() {
            scope
        } else {
            panic!("no scopes")
        }
    }

    pub fn current_scope_type(&self) -> &SymbolTableType {
        return &self.current_scope().symbol_table_type;
    }

    /// Retuns scopes until the given position
    /// the scopes are sorted by start position descending
    pub fn innermost_scope(&self, pos: usize) -> Option<&SymbolTableScope> {
        return self
            .all_scopes
            .iter()
            .filter(|scope| scope.start_pos < pos)
            .last();
    }

    /// get innermost scope that contains that line
    /// search for symbol in that scope
    /// if not found search in parent scope
    /// continue until found or no parent scope
    pub fn lookup_in_scope(&self, lookup_request: LookupSymbolRequest) -> Option<&SymbolTableNode> {
        match lookup_request.position {
            Some(pos) => {
                let mut innermost_scope = self.innermost_scope(pos);
                while let Some(scope) = innermost_scope {
                    if let Some(symbol) = scope.symbols.get(&lookup_request.name) {
                        return Some(symbol);
                    }
                    // We reach the global scope
                    if scope.parent.is_none() {
                        break;
                    }
                    innermost_scope = if let Some(parent_id) = scope.parent {
                        self.all_scopes
                            .iter()
                            .filter(|scope| scope.id == parent_id)
                            .last()
                    } else {
                        Some(self.global_scope())
                    }
                }
            }
            None => {
                if let Some(symbol) = self.current_scope().symbols.get(&lookup_request.name) {
                    return Some(symbol);
                }
            }
        }
        if let Some(symbol) = self.current_scope().symbols.get(&lookup_request.name) {
            return Some(symbol);
        }
        None
    }

    pub fn lookup_in_builtin_scope(&self, name: &str) -> Option<&SymbolTableNode> {
        let builtin_scope = self.get_builtin_scope();
        builtin_scope.symbols.get(name)
    }

    pub fn enter_scope(&mut self, new_scope: SymbolTableScope) {
        self.scopes.push(new_scope);
    }

    pub fn global_scope(&self) -> &SymbolTableScope {
        self.scopes
            .iter()
            .filter(|scope| scope.symbol_table_type == SymbolTableType::Module)
            .last()
            .unwrap()
    }

    pub fn exit_scope(&mut self) {
        let finished_scope = self.scopes.pop();
        match finished_scope {
            Some(scope) => self.all_scopes.push(scope),
            None => panic!("tried to exit non-existent scope"),
        }
    }

    pub fn add_symbol(&mut self, mut symbol_node: SymbolTableNode) {
        match self.scopes.last_mut() {
            Some(scope) => {
                if let Some(existing_symbol) = scope.symbols.get(&symbol_node.name) {
                    symbol_node
                        .declarations
                        .extend(existing_symbol.declarations.clone());
                }
                scope.symbols.insert(symbol_node.name.clone(), symbol_node);
            }
            None => panic!("no current scope, there must be a global scope"),
        };
    }

    pub(crate) fn get_builtin_scope(&self) -> &SymbolTableScope {
        // builtin scope always exists
        self.scopes
            .iter()
            .filter(|scope| scope.symbol_table_type == SymbolTableType::BUILTIN)
            .last()
            .unwrap()
    }
}

impl SymbolTableNode {
    pub fn add_declaration(&mut self, decl: Declaration) {
        self.declarations.push(decl);
    }

    pub fn last_declaration(&self) -> Option<&Declaration> {
        self.declarations.last()
    }

    pub fn declaration_until_position(&self, position: usize) -> Option<&Declaration> {
        let mut filtered_declarations = self
            .declarations
            .iter()
            .filter(|decl| decl.declaration_path().node.start < position)
            .collect::<Vec<&Declaration>>();

        filtered_declarations.sort_by(|a, b| {
            a.declaration_path()
                .node
                .start
                .cmp(&b.declaration_path().node.start)
        });

        filtered_declarations.last().copied()
    }
}

// implement display for symbol table and sort the symbols by key

impl std::fmt::Display for SymbolTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "-------------------")?;
        writeln!(f, "global scope:")?;
        let mut sorted_scopes = self.scopes.iter().collect::<Vec<&SymbolTableScope>>();
        sorted_scopes.sort_by(|a, b| a.name.cmp(&b.name));

        for scope in sorted_scopes {
            // Skip printing the builtin scope
            if scope.symbol_table_type == SymbolTableType::BUILTIN {
                continue;
            }
            writeln!(f, "{}", scope)?;
        }

        writeln!(f, "all scopes:")?;

        let mut sorted_all_scopes = self.all_scopes.iter().collect::<Vec<&SymbolTableScope>>();
        sorted_all_scopes.sort_by(|a, b| a.name.cmp(&b.name));
        for scope in sorted_all_scopes {
            // Skip printing the builtin scope
            // Maybe we can make this a flag
            // Most of the time we don't care about the builtin scope
            if scope.symbol_table_type == SymbolTableType::BUILTIN {
                continue;
            }
            writeln!(f, "{}", scope)?;
        }
        writeln!(f, "-------------------")?;
        Ok(())
    }
}

impl std::fmt::Display for SymbolTableScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sorted_symbols = self
            .symbols
            .iter()
            .collect::<Vec<(&String, &SymbolTableNode)>>();
        sorted_symbols.sort_by(|a, b| a.0.cmp(b.0));

        writeln!(f, "Symbols: in {} (id: {})", self.name, self.id)?;
        for (name, symbol) in sorted_symbols {
            writeln!(f, "{}", name)?;
            // sort the declarations by line number
            let mut sorted_declarations = symbol.declarations.clone();
            sorted_declarations.sort_by(|a, b| {
                a.declaration_path()
                    .node
                    .start
                    .cmp(&b.declaration_path().node.start)
            });

            writeln!(f, "- Declarations:")?;

            for declaration in sorted_declarations {
                writeln!(f, "--:   {}", declaration)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for Declaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Declaration::Variable(v) => write!(f, "{:#?}", v),
            Declaration::Function(fun) => write!(f, "{:#?}", fun),
            Declaration::Class(c) => write!(f, "{:#?}", c),
            Declaration::Parameter(p) => write!(f, "{:#?}", p),
            Declaration::Alias(a) => write!(f, "{:#?}", a),
            Declaration::TypeParameter(t) => write!(f, "{:#?}", t),
            Declaration::TypeAlias(t) => write!(f, "{:#?}", t),
        }
    }
}
