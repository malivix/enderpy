use std::{panic, vec};

use miette::Result;

use super::{
    expression::{is_atom, is_iterable},
    operator::{is_bin_arithmetic_op, is_comparison_operator, is_unary_op, map_unary_operator},
    statement::is_at_compound_statement,
    string::concat_string_exprs,
};
use crate::{
    error::ParsingError,
    lexer::Lexer,
    parser::{
        ast::*,
        string::{extract_string_inside, is_string},
    },
    token::{Kind, Token, TokenValue},
};

#[allow(unused)]
#[derive(Debug)]
pub struct Parser {
    source: String,
    lexer: Lexer,
    cur_token: Token,
    prev_token_end: usize,
    // This var keeps track of how many levels deep we are in a list, tuple or set
    // expression. This is used to determine if we should parse comma separated
    // expressions as tuple or not.
    // This is incremented when we see an opening bracket and decremented when we
    // see a closing bracket.
    nested_expression_list: usize,
    pub errors: Vec<ParsingError>,
    curr_line_string: String,
    curr_line_number: u32,
    curr_line_offset: usize,
    path: String,
}

#[allow(unused)]
impl Parser {
    pub fn new(source: String, path: String) -> Self {
        let mut lexer = Lexer::new(&source);
        let cur_token = lexer.next_token();
        let prev_token_end = 0;

        Self {
            source,
            lexer,
            cur_token,
            prev_token_end,
            nested_expression_list: 0,
            errors: vec![],
            curr_line_string: String::new(),
            path,
            curr_line_number: 1,
            curr_line_offset: 0,
        }
    }

    pub fn parse(&mut self) -> Module {
        let node = self.start_node();
        let mut body = vec![];
        while self.cur_kind() != Kind::Eof {
            // TODO: comments can be parsed and used
            // Also comments can be everywhere
            if self.eat(Kind::Comment) {
                continue;
            }
            if self.at(Kind::NewLine) {
                self.bump_any();
                continue;
            }
            let stmt = if is_at_compound_statement(self.cur_token()) {
                self.parse_compount_statement()
            } else {
                self.parse_simple_statement()
            };
            match stmt {
                Ok(stmt) => body.push(stmt),
                Err(err) => {
                    self.errors.push(err);
                    // It's better to exit on the first error otherwise we will get
                    // a lot of errors that are not relevant
                    // TODO: Implement resilient parsing
                    break;
                }
            }
        }

        Module {
            node: self.finish_node(node),
            body,
        }
    }

    fn start_node(&self) -> Node {
        let token = self.cur_token();
        Node::new(token.start, 0)
    }

    fn finish_node(&self, node: Node) -> Node {
        Node::new(node.start, self.prev_token_end)
    }
    fn cur_token(&self) -> &Token {
        &self.cur_token
    }

    fn cur_kind(&self) -> Kind {
        self.cur_token.kind
    }

    fn peek_token(&mut self) -> Result<Token, ParsingError> {
        let token = self.lexer.peek_token();
        if matches!(token.kind, Kind::Error) {
            let pos = self.cur_token.end;
            let line_number = self.get_line_number_of_character_position(pos);
            let err = ParsingError::InvalidSyntax {
                msg: Box::from(format!("Syntax error: {:?}", token.value)),
                input: self.curr_line_string.clone(),
                advice: "".to_string(),
                span: self.get_span_on_line(pos, pos),
            };
            return Err(err);
        }
        Ok(token)
    }

    fn peek_kind(&mut self) -> Result<Kind, ParsingError> {
        let token = self.peek_token()?;
        Ok(token.kind)
    }

    /// Checks if the current index has token `Kind`
    fn at(&self, kind: Kind) -> bool {
        self.cur_kind() == kind
    }

    /// Advance if we are at `Kind`
    fn bump(&mut self, kind: Kind) {
        if self.at(kind) {
            self.advance();
        }
    }

    /// Advance any token
    fn bump_any(&mut self) {
        self.advance();
    }

    /// Advance and return true if we are at `Kind`, return false otherwise
    fn eat(&mut self, kind: Kind) -> bool {
        if self.at(kind) {
            self.advance();
            return true;
        }
        false
    }

    /// Move to the next token
    fn advance(&mut self) {
        let token = self.lexer.next_token();
        if self.at(Kind::NewLine) {
            self.curr_line_string.clear();
            self.curr_line_number += 1;
        } else {
            self.curr_line_string
                .push_str(&self.source[self.prev_token_end..self.cur_token.end]);
        }
        self.curr_line_offset = self.cur_token.end;
        {
            self.prev_token_end = self.cur_token.end;
            self.cur_token = token;
            if self.cur_kind() == Kind::Comment {
                self.bump(Kind::Comment);
            }
        }
    }

    // TODO: Convert this to a into trait
    fn convert_lexer_error_to_parse(&mut self) -> ParsingError {
        let token = self.cur_token();
        ParsingError::InvalidSyntax {
            msg: token.value.to_string().into(),
            input: self.curr_line_string.clone(),
            advice: "".to_string(),
            span: self.get_span_on_line(token.start, token.end),
        }
    }

    fn advance_to_next_line_or_semicolon(&mut self) {
        while !self.eat(Kind::NewLine) && !self.eat(Kind::SemiColon) && !self.at(Kind::Eof) {
            self.advance();
        }
    }

    /// Expect a `Kind` or return error
    pub fn expect(&mut self, kind: Kind) -> Result<(), ParsingError> {
        if self.at(Kind::Error) {
            let err = self.convert_lexer_error_to_parse();
            self.advance_to_next_line_or_semicolon();
            return Err(err);
        }
        if !self.at(kind) {
            let found = self.cur_token.kind;
            let node = self.start_node();
            let range = self.finish_node(node);
            let err = ParsingError::InvalidSyntax {
                msg: Box::from(format!("Expected {:?} but found {:?}", kind, found)),
                input: self.curr_line_string.clone(),
                advice: "maybe you forgot to put this character".to_string(),
                span: self.get_span_on_line(range.start, range.end),
            };
            self.advance_to_next_line_or_semicolon();
            return Err(err);
        }
        self.bump_any();
        Ok(())
    }

    /// Expect any of `Kinds` or return error
    pub fn expect_any(&mut self, kind: Vec<Kind>) -> Result<(), ParsingError> {
        if self.at(Kind::Error) {
            let err = self.convert_lexer_error_to_parse();
            self.advance_to_next_line_or_semicolon();
            return Err(err);
        }
        if !kind.contains(&self.cur_token.kind) {
            let found = self.cur_token.kind;
            let node = self.start_node();
            let range = self.finish_node(node);
            let mut expected = String::new();
            for kind in kind {
                expected.push_str(&format!("{:?}, ", kind));
            }
            let err = ParsingError::InvalidSyntax {
                msg: Box::from(format!(
                    "Expected one of {:?} but found {:?}",
                    expected, found
                )),
                input: self.curr_line_string.clone(),
                advice: "maybe you forgot to put this character".to_string(),
                span: self.get_span_on_line(range.start, range.end),
            };
            self.advance_to_next_line_or_semicolon();
            return Err(err);
        }
        self.bump_any();
        Ok(())
    }

    // deprecated
    fn unepxted_token(&mut self, node: Node, kind: Kind) -> Result<(), ParsingError> {
        if kind == Kind::Error {
            let err = self.convert_lexer_error_to_parse();
            self.bump_any();
            return Err(err);
        }
        self.bump_any();
        let range = self.finish_node(node);
        let line_number = self.get_line_number_of_character_position(range.start);
        let err = ParsingError::InvalidSyntax {
            msg: Box::from(format!("Unexpected token {:?}", kind)),
            input: self.curr_line_string.clone(),
            advice: String::new(),
            span: self.get_span_on_line(range.start, range.end),
        };
        Err(err)
    }

    fn unexpected_token_new(&mut self, node: Node, kinds: Vec<Kind>, advice: &str) -> ParsingError {
        let curr_kind = self.cur_kind();
        if curr_kind == Kind::Error {
            let err = self.convert_lexer_error_to_parse();
            self.advance_to_next_line_or_semicolon();
            return err;
        }
        self.bump_any();
        let range = self.finish_node(node);
        let line_number = self.curr_line_number;
        let mut expected = String::new();
        for kind in kinds {
            expected.push_str(&format!("{:?}, ", kind));
        }

        ParsingError::InvalidSyntax {
            msg: Box::from(format!(
                "Expected one of {:?} but found {:?}",
                expected,
                self.cur_kind()
            )),
            input: self.curr_line_string.clone(),
            advice: advice.to_string(),
            span: self.get_span_on_line(range.start, range.end),
        }
    }

    fn get_line_number_of_character_position(&self, pos: usize) -> u32 {
        let mut line_number = 1;
        for (i, c) in self.source.chars().enumerate() {
            if i == pos {
                break;
            }
            if c == '\n' {
                line_number += 1;
            }
        }
        line_number
    }

    fn parse_simple_statement(&mut self) -> Result<Statement, ParsingError> {
        let stmt = match self.cur_kind() {
            Kind::Assert => self.parse_assert_statement(),
            Kind::Pass => self.parse_pass_statement(),
            Kind::Del => self.parse_del_statement(),
            Kind::Return => self.parse_return_statement(),
            // https://docs.python.org/3/reference/simple_stmts.html#the-yield-statement
            Kind::Yield => Ok(Statement::ExpressionStatement(
                self.parse_yield_expression()?,
            )),
            Kind::Raise => self.parse_raise_statement(),
            Kind::Break => self.parse_break_statement(),
            Kind::Continue => self.parse_continue_statement(),
            Kind::Import => self.parse_import_statement(),
            Kind::From => self.parse_from_import_statement(),
            Kind::Global => self.parse_global_statement(),
            Kind::Nonlocal => self.parse_nonlocal_statement(),
            _ => {
                if self.cur_kind() == Kind::Identifier
                    && self.cur_token().value.to_string() == "type"
                {
                    self.parse_type_alias_statement()
                } else if self.cur_kind() == Kind::Indent {
                    let node = self.start_node();
                    let kind = self.cur_kind();
                    return Err(self.unexpected_token_new(
                        node,
                        vec![
                            Kind::Assert,
                            Kind::Pass,
                            Kind::Del,
                            Kind::Return,
                            Kind::Yield,
                            Kind::Raise,
                            Kind::Break,
                            Kind::Continue,
                            Kind::Import,
                            Kind::From,
                            Kind::Global,
                            Kind::Nonlocal,
                        ],
                        "Unexpted indent",
                    ));
                } else {
                    self.parse_assignment_or_expression_statement()
                }
            }
        }?;

        self.err_if_statement_not_ending_in_new_line_or_semicolon(stmt.get_node(), stmt.clone());

        Ok(stmt)
    }

    fn parse_compount_statement(&mut self) -> Result<Statement, ParsingError> {
        let stmt = match self.cur_kind() {
            Kind::If => self.parse_if_statement(),
            Kind::While => self.parse_while_statement(),
            Kind::For => self.parse_for_statement(),
            Kind::Try => self.parse_try_statement(),
            Kind::With => self.parse_with_statement(),
            Kind::Def => self.parse_function_definition(vec![]),
            Kind::MatrixMul => self.parse_decorated_function_def_or_class_def(),
            Kind::Class => self.parse_class_definition(vec![]),
            // match is a soft keyword
            // https://docs.python.org/3/reference/lexical_analysis.html#soft-keywords
            Kind::Identifier if self.cur_token().value.to_string() == "match" => {
                self.parse_match_statement()
            }
            Kind::Async => {
                if matches!(self.peek_kind(), Ok(Kind::Def)) {
                    self.parse_function_definition(vec![])
                } else if matches!(self.peek_kind(), Ok(Kind::For)) {
                    self.parse_for_statement()
                } else if matches!(self.peek_kind(), Ok(Kind::With)) {
                    self.parse_with_statement()
                } else {
                    let node = self.start_node();
                    let kind = self.cur_kind();
                    self.bump_any();
                    Err(self.unepxted_token(node, kind).err().unwrap())
                }
            }
            _ => {
                let range = self.finish_node(self.start_node());
                Err(ParsingError::InvalidSyntax {
                    msg: Box::from("Expected compound statement"),
                    input: self.curr_line_string.clone(),
                    advice: "maybe you forgot to put this character".to_string(),
                    span: self.get_span_on_line(range.start, range.end),
                })
            }
        };

        stmt
    }

    fn err_if_statement_not_ending_in_new_line_or_semicolon(
        &mut self,
        node: Node,
        stmt: Statement,
    ) {
        while self.eat(Kind::WhiteSpace) || self.eat(Kind::Comment) {}

        if !matches!(self.cur_kind(), Kind::NewLine | Kind::SemiColon | Kind::Eof) {
            let node = self.finish_node(node);
            let kind = self.cur_kind();
            let err = ParsingError::InvalidSyntax {
                msg: Box::from("Statement does not end in new line or semicolon"),
                input: self.curr_line_string.clone(),
                advice: "Split the statements into two seperate lines or add a semicolon"
                    .to_string(),
                span: self.get_span_on_line(node.start, node.end),
            };
            self.errors.push(err);
        }
    }

    fn parse_if_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.expect(Kind::If)?;
        let test = Box::new(self.parse_named_expression()?);
        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;
        let mut orelse: Option<If> = None;
        while self.at(Kind::Elif) {
            let elif_node = self.start_node();
            self.bump(Kind::Elif);
            let elif_test = Box::new(self.parse_named_expression()?);
            self.expect(Kind::Colon)?;
            let body = self.parse_suite()?;
            let if_value = If {
                node: self.finish_node(elif_node),
                test: elif_test,
                body,
                orelse: vec![],
            };
            if let Some(val) = &mut orelse {
                val.update_orelse(vec![Statement::IfStatement(if_value)]);
            } else {
                orelse = Some(if_value);
            }
        }

        let mut single_else_body: Option<Vec<Statement>> = None;
        if self.eat(Kind::Else) {
            self.expect(Kind::Colon)?;
            let else_body = self.parse_suite()?;
            if let Some(val) = &mut orelse {
                val.update_orelse(else_body);
            } else {
                single_else_body = Some(else_body);
            }
        };

        // if we had any else or elif statements, we need to wrap them in a vec
        // otherwise we just return an empty vec as the else block of the if statement
        let or_else_vec = if let Some(val) = orelse {
            vec![Statement::IfStatement(val)]
        } else {
            match single_else_body {
                Some(val) => val,
                None => vec![],
            }
        };

        Ok(Statement::IfStatement(If {
            node: self.finish_node(node),
            test,
            body,
            orelse: or_else_vec,
        }))
    }

    fn parse_while_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::While);
        let test = Box::new(self.parse_named_expression()?);
        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;
        let orelse = if self.at(Kind::Else) {
            self.bump(Kind::Else);
            self.expect(Kind::Colon)?;
            self.parse_suite()?
        } else {
            vec![]
        };

        Ok(Statement::WhileStatement(While {
            node: self.finish_node(node),
            test,
            body,
            orelse,
        }))
    }

    fn parse_for_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        let is_async = self.eat(Kind::Async);
        self.bump(Kind::For);
        let target = Box::new(self.parse_target_list()?);
        self.expect(Kind::In)?;
        // TODO: I think this would not work for:
        // for a in [1, 2, 3]:
        let iter_list = self.parse_starred_list(Kind::Colon)?;
        let iter = match iter_list.len() {
            0 => {
                return Err(self.unexpected_token_new(node, vec![], "Expected expression"));
            }
            1 => Box::new(iter_list.into_iter().next().unwrap()),
            _ => Box::new(Expression::Tuple(Box::new(Tuple {
                node: self.finish_node(node),
                elements: iter_list,
            }))),
        };

        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;
        let orelse = if self.eat(Kind::Else) {
            self.expect(Kind::Colon)?;
            self.parse_suite()?
        } else {
            vec![]
        };

        if is_async {
            Ok(Statement::AsyncForStatement(AsyncFor {
                node: self.finish_node(node),
                target,
                iter,
                body,
                orelse,
            }))
        } else {
            Ok(Statement::ForStatement(For {
                node: self.finish_node(node),
                target,
                iter,
                body,
                orelse,
            }))
        }
    }

    fn parse_with_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        let is_async = self.eat(Kind::Async);
        self.bump(Kind::With);
        let items = self.parse_with_items()?;
        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;

        if is_async {
            Ok(Statement::AsyncWithStatement(AsyncWith {
                node: self.finish_node(node),
                items,
                body,
            }))
        } else {
            Ok(Statement::WithStatement(With {
                node: self.finish_node(node),
                items,
                body,
            }))
        }
    }

    fn parse_with_items(&mut self) -> Result<Vec<WithItem>, ParsingError> {
        let mut items = vec![];

        if self.eat(Kind::LeftParen) {
            items.push(self.parse_with_item()?);
            while self.eat(Kind::Comma) & !self.at(Kind::RightParen) {
                items.push(self.parse_with_item()?);
            }
            self.expect(Kind::RightParen)?;
            return Ok(items);
        }
        items.push(self.parse_with_item()?);
        while self.eat(Kind::Comma) & !self.at(Kind::Colon) {
            items.push(self.parse_with_item()?);
        }

        Ok(items)
    }

    fn parse_with_item(&mut self) -> Result<WithItem, ParsingError> {
        let node = self.start_node();
        let context_expr = Box::new(self.parse_expression_2()?);
        let optional_vars = if self.eat(Kind::As) {
            Some(Box::new(self.parse_target()?))
        } else {
            None
        };

        Ok(WithItem {
            node: self.finish_node(node),
            context_expr,
            optional_vars,
        })
    }

    fn parse_try_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        let mut is_try_star = false;
        self.expect(Kind::Try);
        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;
        let handlers = if self.at(Kind::Except) {
            if matches!(self.peek_kind(), Ok(Kind::Mul)) {
                is_try_star = true;
            }
            self.parse_except_clauses()?
        } else {
            vec![]
        };
        let orelse = if self.eat(Kind::Else) {
            self.expect(Kind::Colon)?;
            self.parse_suite()?
        } else {
            vec![]
        };

        let finalbody = if self.eat(Kind::Finally) {
            self.expect(Kind::Colon)?;
            self.parse_suite()?
        } else {
            vec![]
        };

        if is_try_star {
            Ok(Statement::TryStarStatement(TryStar {
                node: self.finish_node(node),
                body,
                handlers,
                orelse,
                finalbody,
            }))
        } else {
            Ok(Statement::TryStatement(Try {
                node: self.finish_node(node),
                body,
                handlers,
                orelse,
                finalbody,
            }))
        }
    }

    fn parse_except_clauses(&mut self) -> Result<Vec<ExceptHandler>, ParsingError> {
        let mut handlers = vec![];
        while self.at(Kind::Except) {
            let node = self.start_node();
            self.bump(Kind::Except);
            self.bump(Kind::Mul);
            let typ = if !self.at(Kind::Colon) {
                Some(Box::new(self.parse_expression_2()?))
            } else {
                None
            };
            let name = if self.eat(Kind::As) {
                let val = Some(self.cur_token().value.to_string());
                self.bump(Kind::Identifier);
                val
            } else {
                None
            };

            self.expect(Kind::Colon)?;
            let body = self.parse_suite()?;

            handlers.push(ExceptHandler {
                node: self.finish_node(node),
                typ,
                name,
                body,
            });
        }

        Ok(handlers)
    }

    fn parse_function_definition(
        &mut self,
        decorators: Vec<Expression>,
    ) -> Result<Statement, ParsingError> {
        // TODO: node excludes decorators
        // later we can extract the node for first decorator
        // and start the node from there
        let node = self.start_node();
        let is_async = self.eat(Kind::Async);
        self.expect(Kind::Def)?;
        let name = self.cur_token().value.to_string();
        self.expect(Kind::Identifier)?;
        let type_params = if self.at(Kind::LeftBrace) {
            self.parse_type_parameters()?
        } else {
            vec![]
        };
        self.expect(Kind::LeftParen)?;
        let args = self.parse_parameters(false)?;
        self.expect(Kind::RightParen)?;

        let return_type = if self.eat(Kind::Arrow) {
            Some(Box::new(self.parse_expression_2()?))
        } else {
            None
        };

        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;
        if is_async {
            Ok(Statement::AsyncFunctionDef(AsyncFunctionDef {
                node: self.finish_node(node),
                name,
                args,
                body,
                decorator_list: decorators,
                returns: return_type,
                type_comment: None,
                type_params,
            }))
        } else {
            Ok(Statement::FunctionDef(FunctionDef {
                node: self.finish_node(node),
                name,
                args,
                body,
                decorator_list: decorators,
                returns: return_type,
                // TODO: type comment
                type_comment: None,
                type_params,
            }))
        }
    }

    fn parse_decorated_function_def_or_class_def(&mut self) -> Result<Statement, ParsingError> {
        let mut decorators = vec![];
        while self.eat(Kind::MatrixMul) {
            let name = self.parse_named_expression()?;
            decorators.push(name);
            self.bump(Kind::NewLine);
        }

        if self.at(Kind::Def) || self.at(Kind::Async) {
            self.parse_function_definition(decorators)
        } else {
            self.parse_class_definition(decorators)
        }
    }

    fn parse_class_definition(
        &mut self,
        decorators: Vec<Expression>,
    ) -> Result<Statement, ParsingError> {
        // TODO: node excludes decorators
        // later we can extract the node for first decorator
        // and start the node from there
        let node = self.start_node();
        self.expect(Kind::Class)?;
        let name = self.cur_token().value.to_string();
        self.expect(Kind::Identifier)?;
        let type_params = if self.at(Kind::LeftBrace) {
            self.parse_type_parameters()?
        } else {
            vec![]
        };
        let (bases, keywords) = if self.eat(Kind::LeftParen) {
            let (bases, keywords) = self.parse_argument_list()?;
            self.expect(Kind::RightParen)?;
            (bases, keywords)
        } else {
            (vec![], vec![])
        };
        self.expect(Kind::Colon)?;
        let body = self.parse_suite()?;

        Ok(Statement::ClassDef(ClassDef {
            node: self.finish_node(node),
            name,
            bases,
            keywords,
            body,
            decorator_list: decorators,
            type_params,
        }))
    }

    // https://peps.python.org/pep-0622/#appendix-a-full-grammar
    fn parse_match_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        // This identifier is match word
        // match is a soft keyword
        self.bump(Kind::Identifier);
        let subject = Box::new(self.parse_subject()?);
        self.expect(Kind::Colon)?;
        self.expect(Kind::NewLine)?;
        self.expect(Kind::Indent)?;
        let cases = self.parse_cases()?;
        self.expect_any(vec![Kind::Dedent, Kind::Eof])?;

        Ok(Statement::Match(Match {
            node: self.finish_node(node),
            subject,
            cases,
        }))
    }

    // This is inaccuracy, but I don't know how
    // the grammar should be
    fn parse_subject(&mut self) -> Result<Expression, ParsingError> {
        self.parse_star_named_expressions()
    }

    // star named expresison is similar to starred expression
    // but it does not accept expression as a value
    // https://docs.python.org/3/reference/grammar.html
    fn parse_star_named_expression(&mut self) -> Result<Expression, ParsingError> {
        if self.at(Kind::Mul) {
            self.parse_or_expr()
        } else {
            self.parse_named_expression()
        }
    }

    fn parse_star_named_expressions(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut exprs = vec![self.parse_star_named_expression()?];
        loop {
            if !self.eat(Kind::Comma) {
                break;
            }
            exprs.push(self.parse_star_named_expression()?);
        }

        if exprs.len() == 1 {
            Ok(exprs.remove(0))
        } else {
            Ok(Expression::Tuple(Box::new(Tuple {
                node: self.finish_node(node),
                elements: exprs,
            })))
        }
    }

    fn parse_cases(&mut self) -> Result<Vec<MatchCase>, ParsingError> {
        let mut cases = vec![];
        loop {
            if self.at(Kind::Dedent) || self.at(Kind::Eof) {
                break;
            }
            let node = self.start_node();

            // This identifier is case word
            self.expect(Kind::Identifier)?;
            let pattern = Box::new(self.parse_patterns()?);
            let guard = if self.at(Kind::If) {
                Some(Box::new(self.parse_guard()?))
            } else {
                None
            };
            self.expect(Kind::Colon)?;
            let body = self.parse_suite()?;
            cases.push(MatchCase {
                node: self.finish_node(node),
                pattern,
                guard,
                body,
            });
        }

        Ok(cases)
    }

    fn parse_guard(&mut self) -> Result<Expression, ParsingError> {
        self.expect(Kind::If)?;
        self.parse_named_expression()
    }

    // https://docs.python.org/3/reference/compound_stmts.html#grammar-token-python-grammar-patterns
    // The open sequence pattern is either a pattern or maybe star pattern
    // Here we expect at least one ( pattern or maybe star pattern )
    fn parse_patterns(&mut self) -> Result<MatchPattern, ParsingError> {
        let mut patterns = self.parse_open_sequence_pattern()?;

        if patterns.len() == 1 {
            Ok(patterns.remove(0))
        } else {
            Ok(MatchPattern::MatchSequence(patterns))
        }
    }

    fn parse_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        let or_pattern = self.parse_or_pattern()?;

        if self.at(Kind::As) {
            let node = self.start_node();
            let name = Some(self.cur_token().value.to_string());
            self.bump(Kind::As);
            Ok(MatchPattern::MatchAs(MatchAs {
                node: self.finish_node(node),
                pattern: Some(Box::new(or_pattern)),
                name,
            }))
        } else {
            Ok(or_pattern)
        }
    }

    fn parse_or_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        let mut patterns = vec![];
        patterns.push(self.parse_closed_pattern()?);
        loop {
            if !self.eat(Kind::BitOr) {
                break;
            }
            patterns.push(self.parse_closed_pattern()?);
        }
        if patterns.len() == 1 {
            Ok(patterns.remove(0))
        } else {
            Ok(MatchPattern::MatchOr(patterns))
        }
    }

    fn parse_closed_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        match self.cur_kind() {
            Kind::LeftParen => self.parse_sequence_pattern(),
            Kind::LeftBrace => self.parse_sequence_pattern(),
            Kind::LeftBracket => self.parse_mapping_pattern(),
            Kind::Identifier => {
                if matches!(self.peek_kind(), Ok(Kind::Dot)) {
                    // TODO: use a way to reuse node from value expression
                    let node = self.start_node();
                    let value = self.parse_attr()?;
                    if self.at(Kind::LeftParen) {
                        self.parse_class_pattern(value)
                    } else {
                        self.parse_value_pattern(value, node)
                    }
                } else if matches!(self.peek_kind(), Ok(Kind::LeftParen)) {
                    let value = self.parse_attr()?;
                    self.parse_class_pattern(value)
                } else {
                    self.parse_capture_or_wildcard_pattern()
                }
        },
            Kind::Integer
            | Kind::Binary
            | Kind::Octal
            | Kind::Hexadecimal
            | Kind::PointFloat
            | Kind::ExponentFloat
            | Kind::ImaginaryInteger
            | Kind::ImaginaryPointFloat
            | Kind::ImaginaryExponentFloat
            | Kind::None
            | Kind::True
            | Kind::False
            | Kind::StringLiteral | Kind::RawBytes | Kind::Bytes
            // The signed numbers are also allowed
            | Kind::Minus | Kind::Plus => {
                self.parse_literal_pattern()

                },
            _ => {
                let node = self.start_node();
                Err(self.unexpected_token_new(
                    node,
                    vec![
                        Kind::LeftParen,
                        Kind::LeftBrace,
                        Kind::LeftBracket,
                        Kind::Identifier,
                        Kind::Integer,
                        Kind::Binary,
                        Kind::Octal,
                        Kind::Hexadecimal,
                        Kind::PointFloat,
                        Kind::ExponentFloat,
                        Kind::ImaginaryInteger,
                        Kind::ImaginaryPointFloat,
                        Kind::ImaginaryExponentFloat,
                        Kind::None,
                        Kind::True,
                        Kind::False,
                        Kind::StringLiteral,
                        Kind::RawBytes,
                        Kind::Bytes,
                        Kind::Minus,
                        Kind::Plus,
                    ],
                    "A match pattern starts with these characters",
                ))
            },
        }
    }

    // https://docs.python.org/3/reference/compound_stmts.html#literal-patterns
    fn parse_literal_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        let node = self.start_node();
        let value = Box::new(self.parse_binary_arithmetic_operation()?);
        Ok(MatchPattern::MatchValue(MatchValue {
            node: self.finish_node(node),
            value,
        }))
    }

    fn parse_capture_or_wildcard_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        let capture_value = self.cur_token().value.to_string().clone();
        let node = self.start_node();
        self.expect(Kind::Identifier)?;
        // TODO: should also accpet as?

        if capture_value == "_" {
            Ok(MatchPattern::MatchAs(MatchAs {
                node: self.finish_node(node),
                name: None,
                pattern: None,
            }))
        } else {
            Ok(MatchPattern::MatchAs(MatchAs {
                node: self.finish_node(node),
                name: Some(capture_value),
                pattern: None,
            }))
        }
    }

    // https://docs.python.org/3/reference/compound_stmts.html#value-patterns
    // This pattern shares the value logic with class pattern
    // so we pass that part to this method
    fn parse_value_pattern(
        &mut self,
        value: Expression,
        node: Node,
    ) -> Result<MatchPattern, ParsingError> {
        Ok(MatchPattern::MatchValue(MatchValue {
            node: self.finish_node(node),
            value: Box::new(value),
        }))
    }

    // This parse attr does not allow anything other than names
    // in contrast to attribute parsing in primary expression
    fn parse_attr(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let value = self.cur_token().value.to_string().clone();
        let mut expr = Ok(Expression::Name(Box::new(Name {
            node: self.finish_node(node),
            id: value,
        })));
        self.expect(Kind::Identifier);
        while self.eat(Kind::Dot) {
            let attr_val = self.cur_token().value.to_string();
            self.expect(Kind::Identifier)?;
            expr = Ok(Expression::Attribute(Box::new(Attribute {
                node: self.finish_node(node),
                value: Box::new(expr?),
                attr: attr_val,
            })));
        }
        expr
    }

    // TODO: This has precedence over sequence pattern but I'm not sure
    // what is the right way to use it.
    fn parse_group_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        self.expect(Kind::LeftParen)?;
        let pattern = self.parse_pattern()?;
        self.expect(Kind::RightParen)?;
        Ok(pattern)
    }

    fn parse_mapping_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        let node = self.start_node();
        self.expect(Kind::LeftBracket)?;
        let mut keys = vec![];
        let mut patterns = vec![];
        let mut rest = None;
        loop {
            if self.eat(Kind::RightBracket) {
                break;
            }
            if self.eat(Kind::Pow) {
                rest = Some(self.cur_token().value.to_string().clone());
                self.bump(Kind::Identifier);
                // consume the trailing comma
                self.bump(Kind::Comma);
                // rest is the last element so we expect the closing bracket
                self.expect(Kind::RightBracket)?;
                break;
            } else {
                // TODO: here we cannot accept all primary expressions
                // but python docs do not have the full list of what is allowed
                // so we just do this for now
                keys.push(self.parse_primary(None)?);
                self.expect(Kind::Colon)?;
                patterns.push(self.parse_pattern()?);
            }

            if !self.at(Kind::RightBracket) {
                self.expect(Kind::Comma)?;
            }
        }

        Ok(MatchPattern::MatchMapping(MatchMapping {
            node: self.finish_node(node),
            keys,
            patterns,
            rest,
        }))
    }

    fn parse_literal_or_value_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        if self.cur_kind() == Kind::Identifier && !matches!(self.peek_kind(), Ok(Kind::Colon)) {
            let node = self.start_node();
            let value = self.parse_attr()?;
            self.parse_value_pattern(value, node)
        } else {
            self.parse_literal_pattern()
        }
    }

    fn parse_class_pattern(
        &mut self,
        class_name: Expression,
    ) -> Result<MatchPattern, ParsingError> {
        let node = self.start_node();
        let class = Box::new(class_name);
        self.expect(Kind::LeftParen)?;
        let mut patterns = vec![];
        let mut kwd_attrs = vec![];
        let mut kwd_patterns = vec![];
        let mut seen_keyword_pattern = false;
        loop {
            if self.eat(Kind::RightParen) {
                break;
            }

            if self.at(Kind::Identifier) && matches!(self.peek_kind(), Ok(Kind::Assign)) {
                seen_keyword_pattern = true;
                kwd_attrs.push(self.cur_token().value.to_string().clone());
                self.bump(Kind::Identifier);
                self.bump(Kind::Assign);
                kwd_patterns.push(self.parse_pattern()?);
            } else {
                if seen_keyword_pattern {
                    return Err(ParsingError::InvalidSyntax {
                        msg: Box::from("Positional arguments cannot come after keyword arguments."),
                        input: self.curr_line_string.clone(),
                        advice: "you can only use arguments in form a=b here.".to_string(),
                        span: self.get_span_on_line(node.start, node.end),
                    });
                }
                patterns.push(self.parse_pattern()?);
            }
            if !self.at(Kind::RightParen) {
                self.expect(Kind::Comma)?;
            }
        }
        Ok(MatchPattern::MatchClass(MatchClass {
            node: self.finish_node(node),
            cls: class,
            patterns,
            kwd_attrs,
            kwd_patterns,
        }))
    }

    fn parse_sequence_pattern(&mut self) -> Result<MatchPattern, ParsingError> {
        let node = self.start_node();
        if self.eat(Kind::LeftBrace) {
            let pattern = self.parse_maybe_sequence_pattern()?;
            self.expect(Kind::RightBrace)?;
            Ok(MatchPattern::MatchSequence(pattern))
        } else if self.eat(Kind::LeftParen) {
            let pattern = self.parse_open_sequence_pattern()?;
            self.expect(Kind::RightParen)?;
            Ok(MatchPattern::MatchSequence(pattern))
        } else {
            Err(self.unexpected_token_new(
                node,
                vec![Kind::LeftBrace, Kind::LeftParen],
                "Write a sequence pattern here",
            ))
        }
    }

    fn parse_open_sequence_pattern(&mut self) -> Result<Vec<MatchPattern>, ParsingError> {
        let mut patterns = vec![];
        patterns.push(self.parse_maybe_star_patern()?);
        loop {
            if !self.eat(Kind::Comma) {
                break;
            }
            patterns.push(self.parse_maybe_star_patern()?);
        }
        Ok(patterns)
    }

    fn parse_maybe_sequence_pattern(&mut self) -> Result<Vec<MatchPattern>, ParsingError> {
        let mut patterns = vec![];
        loop {
            if self.at(Kind::RightBrace) {
                break;
            }
            patterns.push(self.parse_maybe_star_patern()?);
            if !self.at(Kind::RightBrace) {
                self.expect(Kind::Comma)?;
            }
        }
        Ok(patterns)
    }
    fn parse_maybe_star_patern(&mut self) -> Result<MatchPattern, ParsingError> {
        if self.eat(Kind::Mul) {
            self.parse_capture_or_wildcard_pattern()
        } else {
            self.parse_pattern()
        }
    }

    fn parse_assignment_or_expression_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        let lhs = self.parse_expression()?;

        if self.cur_kind() == Kind::Assign {
            self.parse_assignment_statement(node, lhs)
        } else if let Some(op) = self.parse_aug_assign_op() {
            self.parse_aug_assignment_statement(node, lhs, op)
        } else if self.at(Kind::Colon) {
            self.parse_ann_assign_statement(node, lhs)
        } else {
            Ok(Statement::ExpressionStatement(lhs))
        }
    }

    // https://docs.python.org/3/reference/compound_stmts.html#grammar-token-python-grammar-suite
    fn parse_suite(&mut self) -> Result<Vec<Statement>, ParsingError> {
        let stmts = if self.eat(Kind::NewLine) {
            self.consume_whitespace_and_newline();
            self.expect(Kind::Indent)?;
            let mut stmts = vec![];
            while !self.eat(Kind::Dedent) && !self.at(Kind::Eof) {
                if self.eat(Kind::Comment) || self.consume_whitespace_and_newline() {
                    continue;
                }
                let stmt = self.parse_statement()?;
                stmts.extend(stmt);
            }
            Ok(stmts)
        } else {
            let stmt = self.parse_statement_list()?;
            Ok(stmt)
        };
        self.bump(Kind::NewLine);
        stmts
    }

    // https://docs.python.org/3/reference/compound_stmts.html#grammar-token-python-grammar-statement
    fn parse_statement(&mut self) -> Result<Vec<Statement>, ParsingError> {
        if is_at_compound_statement(self.cur_token()) {
            let comp_stmt = self.parse_compount_statement()?;
            Ok(vec![comp_stmt])
        } else {
            self.parse_statement_list()
        }
    }

    // https://docs.python.org/3/reference/simple_stmts.html#grammar-token-python-grammar-stmt-list
    fn parse_statement_list(&mut self) -> Result<Vec<Statement>, ParsingError> {
        let mut stmts = vec![];
        let stmt = self.parse_simple_statement()?;
        stmts.push(stmt);
        while self.eat(Kind::SemiColon) {
            let stmt = self.parse_simple_statement()?;
            stmts.push(stmt);
        }
        Ok(stmts)
    }

    fn parse_del_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Del);
        let expr = self.parse_target_list()?;
        // target list can return a tuple for a,b
        // but in delete a,b the a,b part is not a tuple
        // It's a list of targets
        // so we need to unwrap the tuple if it's a tuple
        let expr = match expr {
            Expression::Tuple(tuple) => tuple.elements,
            _ => vec![expr],
        };
        Ok(Statement::Delete(Delete {
            node: self.finish_node(node),
            targets: expr,
        }))
    }

    fn parse_assignment_statement(
        &mut self,
        start: Node,
        lhs: Expression,
    ) -> Result<Statement, ParsingError> {
        let mut targets = vec![lhs];
        self.bump(Kind::Assign);
        let value = loop {
            let rhs = self.parse_expression()?;
            // if there's an assign after the expression we have multiple targets
            // like a = b = 1
            // so we add the rhs to the targets and continue parsing
            // otherwise we break and return the rhs as the value
            if self.eat(Kind::Assign) {
                targets.push(rhs);
            } else {
                break rhs;
            }
        };
        Ok(Statement::AssignStatement(Assign {
            node: self.finish_node(start),
            targets,
            value,
        }))
    }

    fn parse_aug_assignment_statement(
        &mut self,
        start: Node,
        lhs: Expression,
        op: AugAssignOp,
    ) -> Result<Statement, ParsingError> {
        let value = self.parse_assignment_value()?;

        Ok(Statement::AugAssignStatement(AugAssign {
            node: self.finish_node(start),
            target: lhs,
            op,
            value,
        }))
    }

    fn parse_ann_assign_statement(
        &mut self,
        start: Node,
        lhs: Expression,
    ) -> Result<Statement, ParsingError> {
        self.bump(Kind::Colon);
        let annotation = self.parse_expression_2()?;
        let value = if self.eat(Kind::Assign) {
            Some(self.parse_assignment_value()?)
        } else {
            None
        };
        Ok(Statement::AnnAssignStatement(AnnAssign {
            node: self.finish_node(start),
            target: lhs,
            annotation,
            value,
            // TODO: implement simple
            simple: true,
        }))
    }

    // The value is either expression list or yield expression
    // https://docs.python.org/3/reference/simple_stmts.html#assignment-statements
    fn parse_assignment_value(&mut self) -> Result<Expression, ParsingError> {
        if self.cur_kind() == Kind::Yield {
            return self.parse_yield_expression();
        }
        self.parse_expression_list()
    }

    fn parse_aug_assign_op(&mut self) -> Option<AugAssignOp> {
        let op = match self.cur_kind() {
            Kind::AddAssign => AugAssignOp::Add,
            Kind::SubAssign => AugAssignOp::Sub,
            Kind::MulAssign => AugAssignOp::Mult,
            Kind::DivAssign => AugAssignOp::Div,
            Kind::IntDivAssign => AugAssignOp::FloorDiv,
            Kind::ModAssign => AugAssignOp::Mod,
            Kind::PowAssign => AugAssignOp::Pow,
            Kind::BitAndAssign => AugAssignOp::BitAnd,
            Kind::BitOrAssign => AugAssignOp::BitOr,
            Kind::BitXorAssign => AugAssignOp::BitXor,
            Kind::ShiftLeftAssign => AugAssignOp::LShift,
            Kind::ShiftRightAssign => AugAssignOp::RShift,
            _ => return None,
        };
        self.bump_any();
        Some(op)
    }

    fn parse_assert_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Assert);
        let test = self.parse_expression_2()?;
        let msg = if self.eat(Kind::Comma) {
            Some(self.parse_expression_2()?)
        } else {
            None
        };

        Ok(Statement::Assert(Assert {
            node: self.finish_node(node),
            test,
            msg,
        }))
    }

    fn parse_pass_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Pass);
        Ok(Statement::Pass(Pass {
            node: self.finish_node(node),
        }))
    }

    fn parse_return_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Return);
        let value = if self.at(Kind::NewLine) {
            None
        } else {
            Some(self.parse_expression_list()?)
        };
        Ok(Statement::Return(Return {
            node: self.finish_node(node),
            value,
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-raise-statement
    fn parse_raise_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Raise);
        let exc = if matches!(self.cur_kind(), Kind::NewLine | Kind::Eof) {
            None
        } else {
            Some(self.parse_expression_2()?)
        };
        let cause = if self.eat(Kind::From) {
            Some(self.parse_expression_2()?)
        } else {
            None
        };
        Ok(Statement::Raise(Raise {
            node: self.finish_node(node),
            exc,
            cause,
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-break-statement
    fn parse_break_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Break);
        Ok(Statement::Break(Break {
            node: self.finish_node(node),
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-continue-statement
    fn parse_continue_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Continue);
        Ok(Statement::Continue(Continue {
            node: self.finish_node(node),
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-global-statement
    fn parse_global_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Global);
        let mut names = vec![];
        while self.at(Kind::Identifier) {
            let name = self.cur_token().value.to_string();
            names.push(name);
            self.bump(Kind::Identifier);
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(Statement::Global(Global {
            node: self.finish_node(node),
            names,
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-nonlocal-statement
    fn parse_nonlocal_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Nonlocal);
        let mut names = vec![];
        while self.at(Kind::Identifier) {
            let name = self.cur_token().value.to_string();
            names.push(name);
            self.bump(Kind::Identifier);
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(Statement::Nonlocal(Nonlocal {
            node: self.finish_node(node),
            names,
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-import-statement
    fn parse_import_statement(&mut self) -> Result<Statement, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::Import);
        let mut aliases = vec![];
        while self.at(Kind::Identifier) {
            let node = self.start_node();
            let (module, _) = self.parse_module_name()?;
            let alias = self.parse_alias(module, node);
            aliases.push(alias);

            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(Statement::Import(Import {
            node: self.finish_node(node),
            names: aliases,
        }))
    }

    // https://docs.python.org/3/reference/simple_stmts.html#the-from-import-statement
    fn parse_from_import_statement(&mut self) -> Result<Statement, ParsingError> {
        let import_node = self.start_node();
        self.bump(Kind::From);
        let (module, level) = self.parse_module_name()?;
        self.bump(Kind::Import);
        let mut aliases = vec![];
        if self.eat(Kind::LeftParen) {
            while self.at(Kind::Identifier) {
                let alias_name = self.start_node();
                let name = self.cur_token().value.to_string();
                self.bump(Kind::Identifier);
                let asname = self.parse_alias(name, alias_name);
                aliases.push(asname);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
            self.expect(Kind::RightParen)?;
        } else if self.at(Kind::Identifier) {
            while self.at(Kind::Identifier) {
                let alias_name = self.start_node();
                let name = self.cur_token().value.to_string();
                self.bump(Kind::Identifier);
                let asname = self.parse_alias(name, alias_name);
                aliases.push(asname);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
        } else if self.at(Kind::Mul) {
            aliases.push(self.parse_alias("*".to_string(), self.start_node()));
            self.bump(Kind::Mul);
        } else {
            return Err(self.unexpected_token_new(
               import_node,
               vec![Kind::Identifier, Kind::Mul, Kind::LeftParen],
               "Use * for importing everthing or use () to specify names to import or specify the name you want to import"
           ));
        }
        Ok(Statement::ImportFrom(ImportFrom {
            node: self.finish_node(import_node),
            module,
            names: aliases,
            level,
        }))
    }

    fn parse_alias(&mut self, name: String, node: Node) -> Alias {
        let asname = if self.eat(Kind::As) {
            let alias_name = self.cur_token().value.to_string();
            self.bump(Kind::Identifier);
            Some(alias_name)
        } else {
            None
        };
        Alias {
            node: self.finish_node(node),
            name,
            asname,
        }
    }

    fn parse_module_name(&mut self) -> Result<(String, usize), ParsingError> {
        let mut level = 0;
        while self.at(Kind::Dot) | self.at(Kind::Ellipsis) {
            match self.cur_kind() {
                Kind::Dot => {
                    level += 1;
                }
                Kind::Ellipsis => {
                    level += 3;
                }
                _ => {
                    return Err(self.unexpected_token_new(
                        self.start_node(),
                        vec![Kind::Dot, Kind::Ellipsis],
                        "use . or ... to specify relative import",
                    ));
                }
            }
            self.bump_any();
        }
        let mut module = self.cur_token().value.to_string();
        self.expect(Kind::Identifier);
        while self.eat(Kind::Dot) {
            module.push('.');
            module.push_str(self.cur_token().value.to_string().as_str());
            self.expect(Kind::Identifier);
        }
        Ok((module, level))
    }

    // https://docs.python.org/3/library/ast.html#ast.Expr
    fn parse_expression(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let expr = self.parse_expression_2()?;

        let mut exprs = vec![];
        if self.at(Kind::Comma) {
            exprs.push(expr);
            while self.eat(Kind::Comma) {
                if self.at(Kind::Eof) {
                    break;
                }
                exprs.push(self.parse_expression_2()?);
            }
        } else {
            return Ok(expr);
        }

        Ok(Expression::Tuple(Box::new(Tuple {
            node: self.finish_node(node),
            elements: exprs,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#conditional-expressions
    fn parse_conditional_expression(&mut self) -> Result<Expression, ParsingError> {
        let or_test = self.parse_or_test();
        if self.eat(Kind::If) {
            let test = self.parse_or_test()?;
            self.expect(Kind::Else)?;
            let or_else = self.parse_expression_2()?;
            return Ok(Expression::IfExp(Box::new(IfExp {
                node: self.start_node(),
                test: Box::new(test),
                body: Box::new(or_test?),
                orelse: Box::new(or_else),
            })));
        }

        or_test
    }

    // https://docs.python.org/3/reference/expressions.html#assignment-expressions
    fn parse_named_expression(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        if self.at(Kind::Identifier) && matches!(self.peek_kind()?, Kind::Walrus) {
            let identifier = self.cur_token().value.to_string();
            let mut identifier_node = self.start_node();
            identifier_node = self.finish_node(identifier_node);
            self.expect(Kind::Identifier)?;
            identifier_node = self.finish_node(identifier_node);
            if self.eat(Kind::Walrus) {
                let value = self.parse_expression_2()?;
                return Ok(Expression::NamedExpr(Box::new(NamedExpression {
                    node: self.finish_node(node),
                    target: Box::new(Expression::Name(Box::new(Name {
                        node: identifier_node,
                        id: identifier,
                    }))),
                    value: Box::new(value),
                })));
            }
            return Ok(Expression::Name(Box::new(Name {
                node: identifier_node,
                id: identifier,
            })));
        }

        self.parse_expression_2()
    }

    // https://docs.python.org/3/reference/expressions.html#list-displays
    fn parse_list(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::LeftBrace);
        if self.eat(Kind::RightBrace) {
            return Ok(Expression::List(Box::new(List {
                node: self.finish_node(node),
                elements: vec![],
            })));
        }
        let started_with_star = self.at(Kind::Mul);
        let first_elm = self.parse_star_named_expression()?;
        if !started_with_star
            && self.at(Kind::For)
            && (self.at(Kind::For)
                || self.at(Kind::Async) && matches!(self.peek_kind(), Ok(Kind::For)))
        {
            let generators = self.parse_comp_for()?;
            self.expect(Kind::RightBrace)?;
            return Ok(Expression::ListComp(Box::new(ListComp {
                node: self.finish_node(node),
                element: Box::new(first_elm),
                generators,
            })));
        }
        self.bump(Kind::Comma);
        let rest = self.parse_starred_list(Kind::RightBrace)?;
        let elements = vec![first_elm];
        let elements = elements.into_iter().chain(rest).collect();
        self.expect(Kind::RightBrace)?;
        Ok(Expression::List(Box::new(List {
            node: self.finish_node(node),
            elements,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#parenthesized-forms
    fn parse_paren_form_or_generator(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        self.expect(Kind::LeftParen)?;
        if self.at(Kind::RightParen) {
            return Ok(Expression::Tuple(Box::new(Tuple {
                node: self.finish_node(node),
                elements: vec![],
            })));
        }
        // paren form starts with either an expression or a star expression
        // Generator starts with an expression
        // we need to first check if we have a generator or a paren form
        // we do this by checking if the next expression is assignment expression
        // if not we consume the expression and check if the next token is a for
        // if not we have a paren form
        // The first expression we consume have three cases
        // Either an starred item https://docs.python.org/3/reference/expressions.html#grammar-token-python-grammar-starred_expression
        // or an assignment expression
        let first_expr =
            if self.at(Kind::Identifier) && matches!(self.peek_kind(), Ok(Kind::Walrus)) {
                self.parse_named_expression()?
            } else if self.eat(Kind::Mul) {
                let expr = self.parse_or_expr()?;
                Expression::Starred(Box::new(Starred {
                    node: self.finish_node(node),
                    value: Box::new(expr),
                }))
            } else {
                self.parse_expression_2()?
            };

        if matches!(self.cur_kind(), Kind::For) || matches!(self.peek_kind(), Ok(Kind::For)) {
            let generators = self.parse_comp_for()?;
            self.expect(Kind::RightParen)?;
            return Ok(Expression::Generator(Box::new(Generator {
                node: self.finish_node(node),
                element: Box::new(first_expr),
                generators,
            })));
        }

        let expr = self.parse_starred_expression(node, first_expr)?;
        self.expect(Kind::RightParen)?;
        Ok(expr)
    }

    // https://docs.python.org/3/reference/expressions.html#displays-for-lists-sets-and-dictionaries
    fn parse_comp_for(&mut self) -> Result<Vec<Comprehension>, ParsingError> {
        // if current token is async
        let is_async = self.eat(Kind::Async);

        let mut generators = vec![];
        loop {
            let node = self.start_node();
            self.expect(Kind::For)?;
            let target = self.parse_target_list()?;
            self.expect(Kind::In)?;
            let iter = self.parse_or_test()?;
            let ifs = if self.eat(Kind::If) {
                let mut ifs = vec![];
                loop {
                    ifs.push(self.parse_or_test()?);
                    if !self.eat(Kind::If) {
                        break;
                    }
                }
                ifs
            } else {
                vec![]
            };
            generators.push(Comprehension {
                node: self.finish_node(node),
                target: Box::new(target),
                iter: Box::new(iter),
                ifs,
                is_async,
            });
            if !matches!(self.cur_kind(), Kind::For) {
                break;
            }
        }
        Ok(generators)
    }

    // https://docs.python.org/3/reference/simple_stmts.html#grammar-token-python-grammar-target_list
    fn parse_target_list(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut targets = vec![];
        loop {
            targets.push(self.parse_target()?);
            if !self.eat(Kind::Comma) || matches!(self.cur_kind(), Kind::NewLine | Kind::Eof) {
                break;
            }
        }
        if targets.len() == 1 {
            Ok(targets.remove(0))
        } else {
            Ok(Expression::Tuple(Box::new(Tuple {
                node: self.finish_node(node),
                elements: targets,
            })))
        }
    }

    // https://docs.python.org/3/reference/simple_stmts.html#grammar-token-python-grammar-target
    fn parse_target(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut targets = vec![];
        let target = match self.cur_kind() {
            Kind::Identifier => match self.peek_kind() {
                // TODO: atom cannot be all the atoms like string, number
                Ok(Kind::LeftBrace) => {
                    let atom = self.parse_atom()?;
                    self.parse_subscript(node, atom)?
                }
                Ok(Kind::Dot) => {
                    let atom = self.parse_atom()?;
                    self.parse_atribute_ref(node, atom)?
                }
                _ => {
                    let identifier = self.cur_token().value.to_string();
                    let mut identifier_node = self.start_node();
                    identifier_node = self.finish_node(identifier_node);
                    self.expect(Kind::Identifier)?;
                    identifier_node = self.finish_node(identifier_node);
                    return Ok(Expression::Name(Box::new(Name {
                        node: identifier_node,
                        id: identifier,
                    })));
                }
            },
            Kind::LeftBrace => {
                let mut elements = vec![];
                self.bump(Kind::LeftBrace);
                loop {
                    elements.push(self.parse_target()?);
                    if !self.eat(Kind::Comma) {
                        break;
                    }
                }
                self.expect(Kind::RightBrace)?;
                Expression::List(Box::new(List {
                    node: self.finish_node(node),
                    elements,
                }))
            }
            Kind::LeftParen => {
                let mut targets = vec![];
                self.bump(Kind::LeftParen);
                loop {
                    targets.push(self.parse_target()?);
                    if !self.eat(Kind::Comma) {
                        break;
                    }
                }
                self.expect(Kind::RightParen)?;
                if targets.len() == 1 {
                    targets.pop().unwrap()
                } else {
                    Expression::Tuple(Box::new(Tuple {
                        node: self.finish_node(node),
                        elements: targets,
                    }))
                }
            }
            Kind::Mul => Expression::Starred(Box::new(Starred {
                node: self.finish_node(node),
                value: Box::new(self.parse_target()?),
            })),
            _ => panic!("invalid target"),
        };
        targets.push(target);
        while self.eat(Kind::Comma) {
            // check if current kind can be start of a target
            if !matches!(
                self.cur_kind(),
                Kind::Identifier | Kind::LeftBrace | Kind::LeftParen | Kind::Mul
            ) {
                break;
            }
            let next_target = self.parse_target()?;
            targets.push(next_target);
        }

        if targets.len() == 1 {
            Ok(targets.remove(0))
        } else {
            Ok(Expression::Tuple(Box::new(Tuple {
                node: self.finish_node(node),
                elements: targets,
            })))
        }
    }

    fn parse_dict_or_set(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        self.bump(Kind::LeftBracket);
        if self.eat(Kind::RightBracket) {
            return Ok(Expression::Dict(Box::new(Dict {
                node: self.finish_node(node),
                keys: vec![],
                values: vec![],
            })));
        }
        if self.at(Kind::Pow) {
            // key must be None
            let (key, value) = self.parse_double_starred_kv_pair()?;
            return self.parse_dict(node, key, value);
        }
        let first_key_or_element = self.parse_star_named_expression()?;
        if matches!(
            self.cur_kind(),
            Kind::Comma | Kind::RightBracket | Kind::Async | Kind::For | Kind::Walrus
        ) {
            self.parse_set(node, first_key_or_element)
        } else {
            self.expect(Kind::Colon)?;
            let first_value = self.parse_expression_2()?;
            self.parse_dict(node, Some(first_key_or_element), first_value)
        }
    }

    // https://docs.python.org/3/reference/expressions.html#set-displays
    fn parse_set(&mut self, node: Node, first_elm: Expression) -> Result<Expression, ParsingError> {
        if !matches!(first_elm, Expression::Starred(_))
            && self.at(Kind::For)
            && (self.at(Kind::For)
                || self.at(Kind::Async) && matches!(self.peek_kind(), Ok(Kind::For)))
        {
            let generators = self.parse_comp_for()?;
            self.consume_whitespace_and_newline();
            self.expect(Kind::RightBracket)?;
            return Ok(Expression::SetComp(Box::new(SetComp {
                node: self.finish_node(node),
                element: Box::new(first_elm),
                generators,
            })));
        }
        self.bump(Kind::Comma);
        let rest = self.parse_starred_list(Kind::RightBracket)?;
        let mut elements = vec![first_elm];
        elements.extend(rest);
        self.expect(Kind::RightBracket)?;
        Ok(Expression::Set(Box::new(Set {
            node: self.finish_node(node),
            elements,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#dictionary-displays
    fn parse_dict(
        &mut self,
        node: Node,
        first_key: Option<Expression>,
        first_value: Expression,
    ) -> Result<Expression, ParsingError> {
        if self.at(Kind::For) || self.at(Kind::Async) && matches!(self.peek_kind(), Ok(Kind::For)) {
            let Some(key) = first_key else {
                return Err(ParsingError::InvalidSyntax {
                    msg: Box::from("cannot use ** in dict comprehension"),
                    input: self.curr_line_string.clone(),
                    advice: "".into(),
                    span: self.get_span_on_line(node.start, node.end),
                });
            };

            // make sure the first key is some
            let generators = self.parse_comp_for()?;
            self.expect(Kind::RightBracket)?;
            Ok(Expression::DictComp(Box::new(DictComp {
                node: self.finish_node(node),
                key: Box::new(key),
                value: Box::new(first_value),
                generators,
            })))
        } else {
            // we already consumed the first pair
            // so if there are more pairs we need to consume the comma
            if !self.at(Kind::RightBracket) {
                self.expect(Kind::Comma)?;
                self.consume_whitespace_and_newline();
            }
            let mut keys = match first_key {
                Some(k) => vec![k],
                None => vec![],
            };
            let mut values = vec![first_value];
            while !self.eat(Kind::RightBracket) {
                let (key, value) = self.parse_double_starred_kv_pair()?;

                if let Some(k) = key {
                    keys.push(k)
                }

                values.push(value);
                if !self.at(Kind::RightBracket) {
                    self.expect(Kind::Comma)?;
                    self.consume_whitespace_and_newline();
                }
            }
            Ok(Expression::Dict(Box::new(Dict {
                node: self.finish_node(node),
                keys,
                values,
            })))
        }
    }

    fn parse_double_starred_kv_pair(
        &mut self,
    ) -> Result<(Option<Expression>, Expression), ParsingError> {
        if self.eat(Kind::Pow) {
            let value = self.parse_expression_2()?;
            Ok((None, value))
        } else {
            let key = self.parse_expression_2()?;
            self.expect(Kind::Colon)?;
            let value = self.parse_expression_2()?;
            Ok((Some(key), value))
        }
    }

    fn consume_whitespace_and_newline(&mut self) -> bool {
        let mut consumed = false;
        while matches!(self.cur_kind(), Kind::WhiteSpace | Kind::NewLine) {
            self.bump(self.cur_kind());
            consumed = true;
        }
        consumed
    }

    // https://docs.python.org/3/reference/expressions.html#expression-lists
    // termination_kind is used to know when to stop parsing the list
    // for example to parse a tuple the termination_kind is Kind::RightParen
    // caller is responsible to consume the first & last occurrence of the
    // termination_kind
    fn parse_starred_list(
        &mut self,
        termination_kind: Kind,
    ) -> Result<Vec<Expression>, ParsingError> {
        let mut expressions = vec![];
        while !self.at(Kind::Eof) && !self.at(termination_kind) {
            if self.eat(Kind::Comment) || self.consume_whitespace_and_newline() {
                continue;
            }
            let expr = self.parse_starred_item()?;
            if !self.at(Kind::Eof) && !self.at(termination_kind) {
                self.expect(Kind::Comma)?;
            }
            expressions.push(expr);
        }
        Ok(expressions)
    }

    // https://docs.python.org/3/reference/expressions.html#expression-lists
    fn parse_starred_item(&mut self) -> Result<Expression, ParsingError> {
        let mut node = self.start_node();
        if self.eat(Kind::Mul) {
            let starred_value_kind = self.cur_kind();
            let expr = self.parse_or_expr()?;
            node = self.finish_node(node);
            if !is_iterable(&expr) {
                self.unepxted_token(node, starred_value_kind);
            }
            return Ok(Expression::Starred(Box::new(Starred {
                node: self.finish_node(node),
                value: Box::new(expr),
            })));
        }
        self.parse_named_expression()
    }

    // https://docs.python.org/3/reference/expressions.html#conditional-expressions
    fn parse_expression_2(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        if self.eat(Kind::Lambda) {
            let params_list = self.parse_parameters(true).expect("lambda params");
            self.expect(Kind::Colon)?;
            let expr = self.parse_expression_2()?;

            return Ok(Expression::Lambda(Box::new(Lambda {
                node: self.finish_node(node),
                args: params_list,
                body: Box::new(expr),
            })));
        }
        self.parse_conditional_expression()
    }

    // https://docs.python.org/3/reference/expressions.html#boolean-operations
    fn parse_or_test(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let lhs = self.parse_and_test()?;
        if self.eat(Kind::Or) {
            let rhs = self.parse_or_test()?;
            return Ok(Expression::BoolOp(Box::new(BoolOperation {
                node: self.finish_node(node),
                op: BooleanOperator::Or,
                values: vec![lhs, rhs],
            })));
        }
        Ok(lhs)
    }

    // https://docs.python.org/3/reference/expressions.html#boolean-operations
    fn parse_and_test(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let lhs = self.parse_not_test()?;
        if self.at(Kind::And) {
            self.bump(Kind::And);
            let rhs = self.parse_not_test()?;
            return Ok(Expression::BoolOp(Box::new(BoolOperation {
                node: self.finish_node(node),
                op: BooleanOperator::And,
                values: vec![lhs, rhs],
            })));
        }
        Ok(lhs)
    }

    // https://docs.python.org/3/reference/expressions.html#boolean-operations
    fn parse_not_test(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        if self.at(Kind::Not) {
            self.bump(Kind::Not);
            let operand = self.parse_not_test()?;
            return Ok(Expression::UnaryOp(Box::new(UnaryOperation {
                node: self.finish_node(node),
                op: UnaryOperator::Not,
                operand: Box::new(operand),
            })));
        }
        self.parse_comparison()
    }

    // https://docs.python.org/3/reference/expressions.html#comparisons
    fn parse_comparison(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let or_expr = self.parse_or_expr()?;
        let mut ops = vec![];
        let mut comparators = vec![];
        while is_comparison_operator(&self.cur_kind()) {
            ops.push(self.parse_comp_operator()?);
            comparators.push(self.parse_or_expr()?);
        }

        if !ops.is_empty() {
            return Ok(Expression::Compare(Box::new(Compare {
                node: self.finish_node(node),
                left: Box::new(or_expr),
                ops,
                comparators,
            })));
        }

        Ok(or_expr)
    }

    // Binary bitwise operations
    // https://docs.python.org/3/reference/expressions.html#binary-bitwise-operations
    fn parse_or_expr(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut lhs = self.parse_xor_expr()?;
        while self.eat(Kind::BitOr) {
            let rhs = self.parse_xor_expr()?;
            lhs = Expression::BinOp(Box::new(BinOp {
                node: self.finish_node(node),
                op: BinaryOperator::BitOr,
                left: Box::new(lhs),
                right: Box::new(rhs),
            }));
        }
        Ok(lhs)
    }

    // https://docs.python.org/3/reference/expressions.html#binary-bitwise-operations
    fn parse_xor_expr(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut and_expr = self.parse_and_expr()?;
        if self.eat(Kind::BitXor) {
            let rhs = self.parse_and_expr()?;
            and_expr = Expression::BinOp(Box::new(BinOp {
                node: self.finish_node(node),
                op: BinaryOperator::BitXor,
                left: Box::new(and_expr),
                right: Box::new(rhs),
            }));
        }
        Ok(and_expr)
    }

    // https://docs.python.org/3/reference/expressions.html#binary-bitwise-operations
    fn parse_and_expr(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut shift_expr = self.parse_shift_expr()?;

        while self.eat(Kind::BitAnd) {
            let rhs = self.parse_shift_expr()?;
            shift_expr = Expression::BinOp(Box::new(BinOp {
                node: self.finish_node(node),
                op: BinaryOperator::BitAnd,
                left: Box::new(shift_expr),
                right: Box::new(rhs),
            }));
        }
        Ok(shift_expr)
    }

    // https://docs.python.org/3/reference/expressions.html#shifting-operations
    fn parse_shift_expr(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut arith_expr = self.parse_binary_arithmetic_operation()?;
        if self.at(Kind::LeftShift) || self.at(Kind::RightShift) {
            let op = if self.eat(Kind::LeftShift) {
                BinaryOperator::LShift
            } else {
                self.bump(Kind::RightShift);
                BinaryOperator::RShift
            };
            let lhs = self.parse_binary_arithmetic_operation()?;
            arith_expr = Expression::BinOp(Box::new(BinOp {
                node: self.finish_node(node),
                op,
                left: Box::new(arith_expr),
                right: Box::new(lhs),
            }));
        }
        Ok(arith_expr)
    }

    // https://docs.python.org/3/reference/expressions.html#binary-arithmetic-operations
    fn parse_binary_arithmetic_operation(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut lhs = self.parse_unary_arithmetric_operation()?;
        while is_bin_arithmetic_op(&self.cur_kind()) {
            let op = self.parse_bin_arithmetic_op()?;
            let rhs = self.parse_unary_arithmetric_operation()?;
            lhs = Expression::BinOp(Box::new(BinOp {
                node: self.finish_node(node),
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
            }));
        }
        Ok(lhs)
    }

    // https://docs.python.org/3/reference/expressions.html#unary-arithmetic-and-bitwise-operations
    fn parse_unary_arithmetric_operation(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        if is_unary_op(&self.cur_kind()) {
            let op = map_unary_operator(&self.cur_kind());
            self.bump_any();
            let operand = self.parse_unary_arithmetric_operation()?;
            return Ok(Expression::UnaryOp(Box::new(UnaryOperation {
                node: self.finish_node(node),
                op,
                operand: Box::new(operand),
            })));
        }
        self.parse_power_expression()
    }

    // https://docs.python.org/3/reference/expressions.html#the-power-operator
    fn parse_power_expression(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let base = if self.at(Kind::Await) {
            self.bump(Kind::Await);
            let value = self.parse_primary(None)?;
            Ok(Expression::Await(Box::new(Await {
                node: self.finish_node(node),
                value: Box::new(value),
            })))
        } else {
            self.parse_primary(None)
        };
        if self.eat(Kind::Pow) {
            let exponent = self.parse_unary_arithmetric_operation()?;
            return Ok(Expression::BinOp(Box::new(BinOp {
                node: self.finish_node(node),
                op: BinaryOperator::Pow,
                left: Box::new(base?),
                right: Box::new(exponent),
            })));
        }

        base
    }

    // https://docs.python.org/3/reference/expressions.html#primaries
    // primaries can be chained together, when they are chained the base is the
    // previous primary
    fn parse_primary(&mut self, base: Option<Expression>) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut atom_or_primary = if let Some(base) = base {
            base
        } else if is_atom(&self.cur_kind()) {
            self.parse_atom()?
        } else {
            return Err(self.unepxted_token(node, self.cur_kind()).err().unwrap());
        };

        let mut primary = if self.at(Kind::Dot) {
            // TODO: does not handle cases like a.b[0].c
            self.parse_atribute_ref(node, atom_or_primary)
        } else if self.at(Kind::LeftBrace) {
            // https://docs.python.org/3/reference/expressions.html#slicings
            self.parse_subscript(node, atom_or_primary)
        } else if self.eat(Kind::LeftParen) {
            self.bump(Kind::NewLine);
            // parse call
            // https://docs.python.org/3/reference/expressions.html#calls
            let mut positional_args = vec![];
            let mut keyword_args = vec![];
            let mut seen_keyword = false;

            loop {
                if self.at(Kind::RightParen) {
                    break;
                }
                if self.at(Kind::Identifier) && matches!(self.peek_kind(), Ok(Kind::Assign)) {
                    seen_keyword = true;
                    let keyword_arg = self.parse_keyword_item()?;
                    keyword_args.push(keyword_arg);
                } else if self.at(Kind::Mul) {
                    let star_arg_node = self.start_node();
                    self.bump(Kind::Mul);
                    let star_arg = Expression::Starred(Box::new(Starred {
                        node: self.finish_node(star_arg_node),
                        value: Box::new(self.parse_expression_2()?),
                    }));
                    positional_args.push(star_arg);
                } else if self.at(Kind::Pow) {
                    let kwarg_node = self.start_node();
                    self.bump(Kind::Pow);
                    seen_keyword = true;
                    let kwarg = Keyword {
                        node: self.finish_node(kwarg_node),
                        arg: None,
                        value: Box::new(self.parse_expression_2()?),
                    };
                    keyword_args.push(kwarg);
                } else {
                    if seen_keyword {
                        let node_end = self.finish_node(node);
                        return Err(ParsingError::InvalidSyntax {
                            msg: Box::from(
                                "Positional arguments cannot come after keyword arguments.",
                            ),
                            input: self.curr_line_string.clone(),
                            advice: "you can only use arguments in form a=b here.".to_string(),
                            span: self.get_span_on_line(node_end.start, node_end.end),
                        });
                    }
                    let arg = self.parse_named_expression()?;
                    positional_args.push(arg);
                }
                if !self.eat(Kind::Comma) {
                    break;
                }
            }

            self.bump(Kind::Comma);
            self.expect(Kind::RightParen)?;

            Ok(Expression::Call(Box::new(Call {
                node: self.finish_node(node),
                func: Box::new(atom_or_primary),
                args: positional_args,
                keywords: keyword_args,
                starargs: None,
                kwargs: None,
            })))
        } else {
            Ok(atom_or_primary)
        };

        if matches!(
            self.cur_kind(),
            Kind::LeftBrace | Kind::LeftParen | Kind::Dot
        ) {
            primary = self.parse_primary(Some(primary?));
        }

        primary
    }

    // https://docs.python.org/3/reference/expressions.html#grammar-token-python-grammar-argument_list
    // returns args, keywords
    fn parse_argument_list(&mut self) -> Result<(Vec<Expression>, Vec<Keyword>), ParsingError> {
        let mut seen_keyword = false;
        let mut positional_args = vec![];
        let mut keyword_args = vec![];
        loop {
            let node = self.start_node();
            if self.at(Kind::RightParen) {
                break;
            }
            if self.at(Kind::Identifier) && matches!(self.peek_kind(), Ok(Kind::Assign)) {
                seen_keyword = true;
                let keyword_arg = self.parse_keyword_item()?;
                keyword_args.push(keyword_arg);
            } else if self.at(Kind::Mul) {
                let star_arg_node = self.start_node();
                self.bump(Kind::Mul);
                let star_arg = Expression::Starred(Box::new(Starred {
                    node: self.finish_node(star_arg_node),
                    value: Box::new(self.parse_expression_2()?),
                }));
                positional_args.push(star_arg);
            } else if self.at(Kind::Pow) {
                let kwarg_node = self.start_node();
                self.bump(Kind::Pow);
                seen_keyword = true;
                let kwarg = Keyword {
                    node: self.finish_node(kwarg_node),
                    arg: None,
                    value: Box::new(self.parse_expression_2()?),
                };
                keyword_args.push(kwarg);
            } else {
                if seen_keyword {
                    let node_end = self.finish_node(node);
                    return Err(ParsingError::InvalidSyntax {
                        msg: Box::from("Positional arguments cannot come after keyword arguments."),
                        input: self.curr_line_string.clone(),
                        advice: "you can only use arguments in form a=b here.".to_string(),
                        span: self.get_span_on_line(node_end.start, node_end.end),
                    });
                }
                let arg = self.parse_named_expression()?;
                positional_args.push(arg);
            }
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok((positional_args, keyword_args))
    }

    fn parse_atribute_ref(
        &mut self,
        node: Node,
        value: Expression,
    ) -> Result<Expression, ParsingError> {
        let mut expr = Ok(value);
        while self.eat(Kind::Dot) {
            let attr_val = self.cur_token().value.to_string();
            self.expect(Kind::Identifier)?;
            expr = Ok(Expression::Attribute(Box::new(Attribute {
                node: self.finish_node(node),
                value: Box::new(expr?),
                attr: attr_val,
            })));
        }
        expr
    }

    fn parse_subscript(
        &mut self,
        node: Node,
        value: Expression,
    ) -> Result<Expression, ParsingError> {
        let mut expr = Ok(value);
        while self.eat(Kind::LeftBrace) {
            let slice = self.parse_slice_list()?;
            expr = Ok(Expression::Subscript(Box::new(Subscript {
                node: self.finish_node(node),
                value: Box::new(expr?),
                slice: Box::new(slice),
            })));
        }
        expr
    }

    // https://docs.python.org/3/reference/expressions.html#atoms
    fn parse_atom(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        if self.at(Kind::Yield) {
            self.parse_yield_expression()
        } else if self.at(Kind::LeftBrace) {
            self.nested_expression_list += 1;
            let list_expr = self.parse_list();
            self.nested_expression_list -= 1;
            return list_expr;
        } else if self.at(Kind::LeftBracket) {
            self.nested_expression_list += 1;
            let dict_or_set_expr = self.parse_dict_or_set();
            self.nested_expression_list -= 1;
            return dict_or_set_expr;
        } else if self.at(Kind::LeftParen) {
            self.nested_expression_list += 1;
            let tuple_or_named_expr = self.parse_paren_form_or_generator();
            self.nested_expression_list -= 1;
            return tuple_or_named_expr;
        } else if self.at(Kind::Identifier) {
            return self.parse_identifier();
        } else if is_atom(&self.cur_kind()) {
            // value must be cloned to be assigned to the node
            let token_value = self.cur_token().value.clone();
            let token_kind = self.cur_kind();
            // bump needs to happen before creating the atom node
            // otherwise the end would be the same as the start
            self.bump_any();
            let mut expr = self.map_to_atom(node, &token_kind, token_value)?;

            // If the current token is a string, we need to check if there are more strings
            // and concat them
            // https://docs.python.org/3/reference/lexical_analysis.html#string-literal-concatenation
            if is_string(&token_kind) {
                loop {
                    if is_string(&self.cur_kind()) {
                        let token_value = self.cur_token().value.clone();
                        let token_kind = self.cur_kind();
                        self.bump_any();
                        let next_str = self.map_to_atom(node, &token_kind, token_value)?;
                        expr = concat_string_exprs(expr, next_str)?;
                    } else if self.eat(Kind::WhiteSpace) {
                        continue;
                    } else if self.at(Kind::Indent)
                        || self.at(Kind::NewLine)
                        || self.at(Kind::Dedent)
                    {
                        // Normally the strings in two lines are not concatenated
                        // but if they are inside a [], {}, (), they are
                        // here we consume all the indent and newline in this case
                        if self.nested_expression_list > 0 {
                            self.bump_any();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }

            return Ok(expr);
        } else {
            return Err(self.unepxted_token(node, self.cur_kind()).err().unwrap());
        }
    }

    fn parse_identifier(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let value = self.cur_token().value.to_string();
        self.expect(Kind::Identifier)?;
        Ok(Expression::Name(Box::new(Name {
            node: self.finish_node(node),
            id: value,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#yield-expressions
    fn parse_yield_expression(&mut self) -> Result<Expression, ParsingError> {
        let yield_node = self.start_node();
        self.expect(Kind::Yield)?;

        if self.eat(Kind::From) {
            let value = self.parse_expression_2()?;
            return Ok(Expression::YieldFrom(Box::new(YieldFrom {
                node: self.finish_node(yield_node),
                value: Box::new(value),
            })));
        }
        if self.eat(Kind::NewLine) || self.at(Kind::Eof) {
            return Ok(Expression::Yield(Box::new(Yield {
                node: self.finish_node(yield_node),
                value: None,
            })));
        }
        let value = match self.parse_expression_list() {
            Ok(expr) => Some(Box::new(expr)),
            _ => None,
        };
        Ok(Expression::Yield(Box::new(Yield {
            node: self.finish_node(yield_node),
            value,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#expression-lists
    fn parse_expression_list(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut expressions = vec![];
        expressions.push(self.parse_expression_2()?);
        while self.eat(Kind::Comma) && !self.at(Kind::Eof) {
            let expr = self.parse_expression_2()?;
            expressions.push(expr);
        }
        if expressions.len() == 1 {
            return Ok(expressions.pop().unwrap());
        }
        Ok(Expression::Tuple(Box::new(Tuple {
            node: self.finish_node(node),
            elements: expressions,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#expression-lists
    fn parse_starred_expression(
        &mut self,
        node: Node,
        first_elm: Expression,
    ) -> Result<Expression, ParsingError> {
        let mut elements = vec![];
        // if tuple has one element but there's a comma after
        // it, it's a tuple
        let mut seen_comma = false;
        elements.push(first_elm);
        while !self.at(Kind::Eof) && !self.at(Kind::RightParen) {
            self.expect(Kind::Comma)?;
            if self.at(Kind::RightParen) {
                break;
            }
            let expr = self.parse_starred_item()?;
            elements.push(expr);
            seen_comma = true;
        }
        if elements.len() == 1 && !seen_comma {
            return Ok(elements.pop().unwrap());
        }
        Ok(Expression::Tuple(Box::new(Tuple {
            node: self.finish_node(node),
            elements,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#slicings
    // Clsoing will be consumed by this function
    fn parse_slice_list(&mut self) -> Result<Expression, ParsingError> {
        let node = self.start_node();
        let mut elements = vec![];
        // TODO: This EOF check should not be here.
        while !self.at(Kind::Eof) && !self.at(Kind::RightBrace) {
            if self.at(Kind::Colon) {
                elements.push(self.parse_proper_slice(None)?);
            } else {
                let expr = self.parse_expression_2()?;
                if self.at(Kind::Colon) {
                    elements.push(self.parse_proper_slice(Some(expr))?);
                } else {
                    elements.push(expr);
                }
            }
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        self.expect(Kind::RightBrace)?;
        if elements.len() == 1 {
            return Ok(elements.pop().unwrap());
        }
        Ok(Expression::Tuple(Box::new(Tuple {
            node: self.finish_node(node),
            elements,
        })))
    }

    // https://docs.python.org/3/reference/expressions.html#slicings
    fn parse_proper_slice(
        &mut self,
        lower: Option<Expression>,
    ) -> Result<Expression, ParsingError> {
        let node = self.start_node();

        let slice_lower = if let Some(lower) = lower {
            Some(Box::new(lower))
        } else if self.eat(Kind::Colon) {
            None
        } else {
            Some(Box::new(self.parse_expression_2()?))
        };
        let upper = if self.eat(Kind::Colon) {
            if self.at(Kind::RightBrace) || self.at(Kind::Colon) {
                None
            } else {
                Some(Box::new(self.parse_expression_2()?))
            }
        } else {
            None
        };
        let step = if self.eat(Kind::Colon) {
            if self.at(Kind::RightBrace) {
                None
            } else {
                Some(Box::new(self.parse_expression_2()?))
            }
        } else {
            None
        };
        Ok(Expression::Slice(Box::new(Slice {
            node: self.finish_node(node),
            lower: slice_lower,
            upper,
            step,
        })))
    }

    fn map_to_atom(
        &mut self,
        start: Node,
        kind: &Kind,
        value: TokenValue,
    ) -> Result<Expression, ParsingError> {
        let atom = match kind {
            Kind::Identifier => Expression::Name(Box::new(Name {
                node: self.finish_node(start),
                id: value.to_string(),
            })),
            Kind::Integer => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Int(value.to_string()),
            })),
            Kind::None => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::None,
            })),
            Kind::True => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Bool(true),
            })),
            Kind::False => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Bool(false),
            })),
            Kind::ImaginaryInteger => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Complex {
                    real: "0".to_string(),
                    imaginary: value.to_string(),
                },
            })),
            Kind::Bytes => {
                let bytes_val = extract_string_inside(
                    value
                        .to_string()
                        .strip_prefix('b')
                        .expect("bytes literal must start with b")
                        .to_string(),
                )
                .into_bytes();
                Expression::Constant(Box::new(Constant {
                    node: self.finish_node(start),
                    value: ConstantValue::Bytes(bytes_val),
                }))
            }
            Kind::StringLiteral => {
                let string_val = extract_string_inside(value.to_string());
                Expression::Constant(Box::new(Constant {
                    node: self.finish_node(start),
                    value: ConstantValue::Str(string_val),
                }))
            }

            Kind::RawBytes => {
                // rb or br appear in the beginning of raw bytes
                let bytes_val =
                    extract_string_inside(value.to_string().chars().skip(2).collect::<String>())
                        .into_bytes();
                Expression::Constant(Box::new(Constant {
                    node: self.finish_node(start),
                    value: ConstantValue::Bytes(bytes_val),
                }))
            }
            Kind::FStringStart => {
                let fstring = self.parse_fstring()?;
                Expression::JoinedStr(Box::new(JoinedStr {
                    node: self.finish_node(start),
                    values: fstring,
                }))
            }
            Kind::PointFloat => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Float(value.to_string()),
            })),
            Kind::ExponentFloat => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Float(value.to_string()),
            })),
            Kind::ImaginaryPointFloat => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Complex {
                    real: "0".to_string(),
                    imaginary: value.to_string(),
                },
            })),
            Kind::ImaginaryExponentFloat => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Complex {
                    real: "0".to_string(),
                    imaginary: value.to_string(),
                },
            })),
            Kind::Ellipsis => Expression::Constant(Box::new(Constant {
                node: self.finish_node(start),
                value: ConstantValue::Ellipsis,
            })),
            _ => {
                return Err(self.unepxted_token(start, self.cur_kind()).err().unwrap());
            }
        };
        Ok(atom)
    }

    fn parse_comp_operator(&mut self) -> Result<ComparisonOperator, ParsingError> {
        let node = self.start_node();
        let op = match self.cur_kind() {
            Kind::Less => ComparisonOperator::Lt,
            Kind::Greater => ComparisonOperator::Gt,
            Kind::LessEq => ComparisonOperator::LtE,
            Kind::GreaterEq => ComparisonOperator::GtE,
            Kind::Eq => ComparisonOperator::Eq,
            Kind::NotEq => ComparisonOperator::NotEq,
            Kind::In => ComparisonOperator::In,
            Kind::Is => match self.peek_kind() {
                Ok(Kind::Not) => {
                    self.bump_any();
                    ComparisonOperator::IsNot
                }
                _ => ComparisonOperator::Is,
            },
            Kind::Not => match self.peek_kind() {
                Ok(Kind::In) => {
                    self.bump_any();
                    ComparisonOperator::NotIn
                }
                _ => {
                    return Err(self.unepxted_token(node, self.cur_kind()).err().unwrap());
                }
            },
            _ => {
                return Err(self.unepxted_token(node, self.cur_kind()).err().unwrap());
            }
        };
        self.bump_any();
        Ok(op)
    }

    fn parse_bin_arithmetic_op(&mut self) -> Result<BinaryOperator, ParsingError> {
        let op = match self.cur_kind() {
            Kind::Plus => Ok(BinaryOperator::Add),
            Kind::Minus => Ok(BinaryOperator::Sub),
            Kind::Mul => Ok(BinaryOperator::Mult),
            Kind::Div => Ok(BinaryOperator::Div),
            Kind::IntDiv => Ok(BinaryOperator::FloorDiv),
            Kind::Mod => Ok(BinaryOperator::Mod),
            Kind::Pow => Ok(BinaryOperator::Pow),
            Kind::MatrixMul => Ok(BinaryOperator::MatMult),
            _ => Err(self
                .unepxted_token(self.start_node(), self.cur_kind())
                .err()
                .unwrap()),
        };
        self.bump_any();
        op
    }

    fn parse_keyword_item(&mut self) -> Result<Keyword, ParsingError> {
        let node = self.start_node();
        let arg = self.cur_token().value.to_string();
        self.expect(Kind::Identifier);
        self.expect(Kind::Assign);
        let value = Box::new(self.parse_expression_2()?);
        Ok(Keyword {
            node: self.finish_node(node),
            arg: Some(arg),
            value,
        })
    }

    fn parse_parameters(&mut self, is_lambda: bool) -> Result<Arguments, ParsingError> {
        let node = self.start_node();
        let mut seen_vararg = false;
        let mut seen_kwarg = false;
        let mut must_have_default = false;

        let mut posonlyargs = vec![];
        let mut args = vec![];
        let mut vararg = None;
        let mut kwarg = None;
        let mut kwonlyargs = vec![];
        let mut kw_defaults = vec![];
        let mut defaults = vec![];

        loop {
            if self.is_def_parameter() {
                let (param, default) = self.parse_parameter(is_lambda)?;
                if seen_vararg {
                    kwonlyargs.push(param);
                } else if seen_kwarg {
                    return Err(ParsingError::InvalidSyntax {
                        msg: Box::from("positional argument follows keyword argument"),
                        input: self.curr_line_string.clone(),
                        advice: "you can only use arguments in form a=b here.".to_string(),
                        span: self.get_span_on_line(self.cur_token().start, self.cur_token().end),
                    });
                } else {
                    args.push(param);
                }

                // TODO: refactor this
                if seen_vararg {
                    kw_defaults.push(default);
                } else if let Some(default_value) = default {
                    must_have_default = true;
                    defaults.push(default_value);
                } else if must_have_default {
                    return Err(ParsingError::InvalidSyntax {
                        msg: Box::from("non-default argument follows default argument"),
                        input: self.curr_line_string.clone(),
                        advice: "you can only use arguments with default value here.".to_string(),
                        span: self.get_span_on_line(self.cur_token().start, self.cur_token().end),
                    });
                }
            // If a parameter has a default value, all following parameters up
            // until the “*” must also have a default value — this
            // is a syntactic restriction that is not expressed by the grammar.
            } else if self.eat(Kind::Mul) {
                seen_vararg = true;
                // after seeing vararg the must_have_default is reset
                // until we see a default value again
                must_have_default = false;
                let (param, default) = self.parse_parameter(is_lambda)?;
                // default is not allowed for vararg
                if default.is_some() {
                    return Err(ParsingError::InvalidSyntax {
                        msg: Box::from("var-positional argument cannot have default value"),
                        input: self.curr_line_string.clone(),
                        advice: "remove the default value of this argument".to_string(),
                        span: self.get_span_on_line(self.cur_token().start, self.cur_token().end),
                    });
                }
                vararg = Some(param);
            } else if self.eat(Kind::Pow) {
                seen_kwarg = true;
                let (param, default) = self.parse_parameter(is_lambda)?;
                // default is not allowed for kwarg
                if default.is_some() {
                    return Err(ParsingError::InvalidSyntax {
                        msg: Box::from("var-keyword argument cannot have default value"),
                        input: self.curr_line_string.clone(),
                        advice: "remove the default value of this argument".to_string(),
                        span: self.get_span_on_line(self.cur_token().start, self.cur_token().end),
                    });
                }
                kwarg = Some(param);
            } else if self.eat(Kind::Comma) {
                continue;
            } else if self.eat(Kind::Div) {
                // copy the current args to posonlyargs
                posonlyargs = args;
                args = vec![];
            } else {
                break;
            }
        }
        // return Parameter

        Ok(Arguments {
            node: self.finish_node(node),
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            kw_defaults,
            kwarg,
            defaults,
        })
    }

    fn is_def_parameter(&mut self) -> bool {
        self.cur_kind() == Kind::Identifier
        // && matches!(self.peek_kind(), Ok(Kind::Assign))
        // || matches!(self.peek_kind(), Ok(Kind::Colon))
    }

    fn parse_parameter(
        &mut self,
        is_lambda: bool,
    ) -> Result<(Arg, Option<Expression>), ParsingError> {
        let node = self.start_node();
        let arg = self.cur_token().value.to_string();
        self.bump(Kind::Identifier);
        // Lambda parameters cannot have annotations
        let annotation = if self.at(Kind::Colon) && !is_lambda {
            self.bump(Kind::Colon);
            Some(self.parse_expression_2()?)
        } else {
            None
        };
        let default = if self.eat(Kind::Assign) {
            Some(self.parse_expression_2()?)
        } else {
            None
        };
        Ok((
            Arg {
                node: self.finish_node(node),
                arg,
                annotation,
            },
            default,
        ))
    }

    // the FStringStart token is consumed by the caller
    fn parse_fstring(&mut self) -> Result<Vec<Expression>, ParsingError> {
        let mut expressions = vec![];
        while self.cur_kind() != Kind::FStringEnd {
            match self.cur_kind() {
                Kind::FStringMiddle => {
                    let str_val = self.cur_token().value.to_string().clone();
                    self.bump(Kind::FStringMiddle);
                    expressions.push(Expression::Constant(Box::new(Constant {
                        node: self.start_node(),
                        value: ConstantValue::Str(str_val),
                    })));
                }
                Kind::LeftBracket => {
                    self.bump(Kind::LeftBracket);
                    expressions.push(self.parse_expression()?);
                    self.expect(Kind::RightBracket)?;
                }
                _ => {
                    return Err(self
                        .unepxted_token(self.start_node(), self.cur_kind())
                        .err()
                        .unwrap());
                }
            }
        }
        self.bump(Kind::FStringEnd);
        Ok(expressions)
    }

    // This function is just here to make it easier to refactor the spans.
    // I don't know how the spans and line number should be handled here
    fn get_span_on_line(&self, start: usize, end: usize) -> (usize, usize) {
        (start, end)
    }

    // https://docs.python.org/3/reference/compound_stmts.html#type-parameter-lists
    fn parse_type_parameters(&mut self) -> Result<Vec<TypeParam>, ParsingError> {
        let node = self.start_node();
        self.expect(Kind::LeftBrace)?;
        let mut type_params = vec![];
        while !self.eat(Kind::RightBrace) {
            match self.cur_kind() {
                Kind::Identifier => {
                    let node = self.start_node();
                    let name = self.cur_token().value.to_string();
                    self.bump(Kind::Identifier);
                    let bound = if self.eat(Kind::Colon) {
                        Some(self.parse_expression_2()?)
                    } else {
                        None
                    };
                    type_params.push(TypeParam::TypeVar(TypeVar {
                        node: self.finish_node(node),
                        name,
                        bound,
                    }));
                }
                Kind::Pow => {
                    // param spec
                    let node = self.start_node();
                    self.bump(Kind::Pow);
                    let name = self.cur_token().value.to_string();
                    self.bump(Kind::Identifier);
                    type_params.push(TypeParam::ParamSpec(ParamSpec {
                        node: self.finish_node(node),
                        name,
                    }));
                }
                Kind::Mul => {
                    // type var tuple
                    let node = self.start_node();
                    self.bump(Kind::Mul);
                    let name = self.cur_token().value.to_string();
                    self.bump(Kind::Identifier);
                    type_params.push(TypeParam::TypeVarTuple(TypeVarTuple {
                        node: self.finish_node(node),
                        name,
                    }));
                }
                _ => {
                    return Err(self.unexpected_token_new(
                        node,
                        vec![Kind::Identifier, Kind::Pow, Kind::Mul, Kind::RightBracket],
                        "",
                    ));
                }
            }
            if !self.at(Kind::RightBrace) {
                self.expect(Kind::Comma)?;
            }
        }
        if type_params.is_empty() {
            return Err(self.unexpected_token_new(
                node,
                vec![Kind::Identifier, Kind::Pow, Kind::Mul],
                "Type parameter is empty",
            ));
        }
        Ok(type_params)
    }

    fn parse_type_alias_statement(&mut self) -> std::result::Result<Statement, ParsingError> {
        let node = self.start_node();
        self.expect(Kind::Identifier)?;
        let name = self.cur_token().value.to_string();
        self.expect(Kind::Identifier)?;
        let type_params = if self.eat(Kind::LeftBrace) {
            let type_params = self.parse_type_parameters()?;
            self.expect(Kind::RightBrace)?;
            type_params
        } else {
            vec![]
        };
        self.expect(Kind::Assign)?;
        let value = self.parse_expression_2()?;
        Ok(Statement::TypeAlias(TypeAlias {
            node: self.finish_node(node),
            name,
            type_params,
            value: Box::new(value),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use insta::{assert_debug_snapshot, glob};

    use super::*;

    #[test]
    fn test_parse_assignment() {
        for test_case in &[
            "a = 1",
            "a = None",
            "a = True",
            "a = False",
            "a = 1j",
            "a = b'1'",
            "a = rb'1'",
            "a = br'1'",
            "a = \"a\"",
            "a = '''a'''",
            "a = \"\"\"a\"\"\"",
            "a = 'a'",
            "a = 1, 2",
            "a = 1, 2, ",
            "a = b = 1",
            "a,b = c,d = 1,2",
            // augmented assignment
            "a += 1",
            "a -= 1",
            "a *= 1",
            "a /= 1",
            "a //= 1",
            "a %= 1",
            "a **= 1",
            "a <<= 1",
            "a >>= 1",
            "a &= 1",
            "a ^= 1",
            "a |= 1",
            // annotated assignment
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_parse_assert_stmt() {
        for test_case in &["assert a", "assert a, b", "assert True, 'fancy message'"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_pass_stmt() {
        for test_case in &["pass", "pass ", "pass\n"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_parse_del_stmt() {
        for test_case in &["del a", "del a, b", "del a, b, "] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn parse_yield_statement() {
        for test_case in &["yield", "yield a", "yield a, b", "yield a, b, "] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_raise_statement() {
        for test_case in &["raise", "raise a", "raise a from c"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_parse_break_continue() {
        for test_case in &["break", "continue"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_parse_bool_op() {
        for test_case in &["a or b", "a and b", "a or b or c", "a and b or c"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_parse_unary_op() {
        for test_case in &["not a", "+ a", "~ a", "-a"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_named_expression() {
        {
            let test_case = &"(a := b)";
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_tuple() {
        for test_case in &[
            "(a, b, c)",
            "(a,
            b, c)",
            "(a
            , b, c)",
            "(a,
            b,
                c)",
            "(a,
)",
            "(a, b, c,)",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_yield_expression() {
        for test_case in &["yield", "yield a", "yield from a"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_starred() {
        {
            let test_case = &"(*a)";
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_await_expression() {
        {
            let test_case = &"await a";
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_attribute_ref() {
        for test_case in &["a.b", "a.b.c", "a.b_c", "a.b.c.d"] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn parse_call() {
        for test_case in &[
            "a()",
            "a(b)",
            "a(b, c)",
            "func(b=c)",
            "func(a, b=c, d=e)",
            "func(a, b=c, d=e, *f)",
            "func(a, b=c, d=e, *f, **g)",
            "func(a,)",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_lambda() {
        for test_case in &[
            "lambda: a",
            "lambda a: a",
            "lambda a, b: a",
            "lambda a, b, c: a",
            "lambda a, *b: a",
            "lambda a, *b, c: a",
            "lambda a, *b, c, **d: a",
            "lambda a=1 : a",
            "lambda a=1 : a,",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_conditional_expression() {
        {
            let test_case = &"a if b else c if d else e";
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_string_literal_concatnation() {
        for test_case in &[
            "'a' 'b'",
            "b'a' b'b'",
            "'a'   'b'",
            "r'a' 'b'",
            "b'a' 'b'",
            "('a'
            'b')",
            "('a'
            'b', 'c')",
            "('a'
                'b'
'c')",
            "f'a' 'c'",
            "f'a' 'b' 'c'",
            "'d' f'a' 'b'",
            "f'a_{1}' 'b' ",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_fstring() {
        for test_case in &[
            "f'a'",
            "f'hello_{a}'",
            "f'hello_{a} {b}'",
            "f'hello_{a} {b} {c}'",
            // unsupported
            // "f'hello_{f'''{a}'''}'",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_comparison() {
        for test_case in &[
            "a == b",
            "a != b",
            "a > b",
            "a < b",
            "a >= b",
            "a <= b",
            "a is b",
            "a is not b",
            "a in b",
            "a not in b",
            "a < b < c",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                description => test_case.to_string(), // the template source code
                omit_expression => true // do not include the default expression
            }, {
                assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_if() {
        for test_case in &[
            "if a: pass",
            "if a:
    pass",
            "if a:
        a = 1
if a:
        b = 1

",
            "if a:
    pass;pass",
            "if a is b:
            pass",
            "if a is b:
                pass
elif a is c:
                pass",
            "if a is b:
                pass
elif a is c:
                pass
else:
                pass
",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_while_statement() {
        for test_case in &[
            "while a: pass",
            "while a:
    pass",
            "while a:
        a = 1
else:
        b = 1
",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_try_statement() {
        for test_case in &[
            "try:
    pass
except:
    pass",
            "try:
                pass
except Exception:
                pass",
            "try:
                pass
except Exception as e:
                pass",
            "try:
                pass
except Exception as e:
                pass
else:
                pass",
            "try:
                pass
except Exception as e:
                pass
else:
                pass
finally:
                pass",
            "try:
    pass
except *Exception as e:
    pass
",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_ellipsis_statement() {
        for test_case in &[
            "def a(): ...",
            "def a():
    ...",
            "a = ...",
            "... + 1",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_comment() {
        for test_case in &[
            "# a",
            "# a\n",
            "# a\n# b",
            "# a\n# b\n",
            "# a\n# b\n# c",
            "# a\n# b\n# c\n",
            "# a\n# b\n# c\n# d",
            "def a(): ... # a",
            "def a(): ... # a\n",
            "def a():
    ... # a",
            "# this is a comment only  program",
            "if a:

	pass",
        ] {
            let mut parser = Parser::new(test_case.to_string(), String::from(""));
            let program = parser.parse();
            insta::with_settings!({
                    description => test_case.to_string(), // the template source code
                    omit_expression => true // do not include the default expression
                }, {
                    assert_debug_snapshot!(program);
            });
        }
    }

    #[test]
    fn test_complete() {
        glob!("../../test_data", "inputs/*.py", |path| {
            let test_case = fs::read_to_string(path).unwrap();
            let mut parser = Parser::new(
                test_case.clone(),
                String::from(path.file_name().unwrap().to_str().unwrap()),
            );
            let program = parser.parse();

            insta::with_settings!({
                    description => test_case.clone(),
                    omit_expression => true
                }, {
                    assert_debug_snapshot!(program);
            });

            if !parser.errors.is_empty() {
                insta::with_settings!({
                        description => test_case,
                        omit_expression => true
                    }, {
                        assert_debug_snapshot!(parser.errors);
                });
            }
        });
    }

    #[test]
    fn test_one_liners() {
        glob!("../../test_data", "inputs/one_liners/*.py", |path| {
            let input = fs::read_to_string(path).unwrap();
            for test_case in input.split("\n\n") {
                let mut parser = Parser::new(
                    test_case.to_string(),
                    String::from(path.file_name().unwrap().to_str().unwrap()),
                );
                let program = parser.parse();

                insta::with_settings!({
                        description => test_case, // the template source code
                        omit_expression => true // do not include the default expression
                    }, {
                        assert_debug_snapshot!(program);
                });
                if !parser.errors.is_empty() {
                    insta::with_settings!({
                            description => test_case,
                            omit_expression => true
                        }, {
                            assert_debug_snapshot!(parser.errors);
                    });
                }
            }
        });
    }
}
