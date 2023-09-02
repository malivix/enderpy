use parser::ast::{self, BinaryOperator};

use super::types::Type;

pub fn get_type_from_annotation(type_annotation: &ast::Expression) -> Type {
    let expr_type = match type_annotation {
        ast::Expression::Name(name) => match name.id.as_str() {
            "int" => Type::Int,
            "float" => Type::Float,
            "str" => Type::Str,
            "bool" => Type::Bool,
            "None" => Type::None,
            _ => Type::Unknown,
        },
        _ => Type::Unknown,
    };

    expr_type
}

pub fn type_equal(t1: &Type, t2: &Type) -> bool {
    match (t1, t2) {
        (Type::Int, Type::Int) => true,
        (Type::Float, Type::Float) => true,
        (Type::Str, Type::Str) => true,
        (Type::Bool, Type::Bool) => true,
        (Type::None, Type::None) => true,
        _ => false,
    }
}

pub fn type_check_bin_op(t1: &Type, t2: &Type, op: &BinaryOperator) -> bool {
    let check_table = match op {
        BinaryOperator::Add => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
        BinaryOperator::Sub => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
        BinaryOperator::Mult => vec![
            (Type::Int, Type::Int),
            (Type::Float, Type::Float),
            (Type::Str, Type::Int),
            (Type::Int, Type::Str),
        ],
        BinaryOperator::Div => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
        BinaryOperator::Mod => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
        BinaryOperator::Pow => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
        BinaryOperator::LShift => vec![(Type::Int, Type::Int)],
        BinaryOperator::RShift => vec![(Type::Int, Type::Int)],
        BinaryOperator::BitOr => vec![(Type::Int, Type::Int)],
        BinaryOperator::BitAnd => vec![(Type::Int, Type::Int)],
        BinaryOperator::BitXor => vec![(Type::Int, Type::Int)],
        BinaryOperator::FloorDiv => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
        BinaryOperator::MatMult => vec![(Type::Int, Type::Int), (Type::Float, Type::Float)],
    };

    for (t1_, t2_) in check_table {
        if type_equal(t1, &t1_) && type_equal(t2, &t2_) {
            return true;
        }
    }

    false
}

pub fn bin_op_result_type(t1: &Type, t2: &Type, op: &BinaryOperator) -> Type {
    if !type_check_bin_op(t1, t2, op) {
        return Type::Unknown;
    }

    match op {
        BinaryOperator::Add
        | BinaryOperator::Sub
        | BinaryOperator::Mult
        | BinaryOperator::MatMult
        | BinaryOperator::Div
        | BinaryOperator::Mod
        | BinaryOperator::Pow
        | BinaryOperator::LShift
        | BinaryOperator::RShift
        | BinaryOperator::BitOr
        | BinaryOperator::BitXor
        | BinaryOperator::BitAnd
        | BinaryOperator::FloorDiv => {
            if type_equal(t1, &Type::Float) || type_equal(t2, &Type::Float) {
                return Type::Float;
            }
            if type_equal(t1, &Type::Int) || type_equal(t2, &Type::Int) {
                return Type::Int;
            }
            match t1 {
                Type::Str => Type::Str,
                Type::None => Type::None,
                Type::Unknown => Type::Unknown,
                Type::Bool => Type::Bool,
                Type::Int => Type::Int,
                Type::Float => Type::Float,
                _ => Type::Unknown,
            }
        }
    }
}
