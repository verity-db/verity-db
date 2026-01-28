//! SQL parsing for the query engine.
//!
//! Wraps `sqlparser` to parse a minimal SQL subset:
//! - SELECT with column list or *
//! - FROM single table
//! - WHERE with comparison predicates
//! - ORDER BY
//! - LIMIT

use sqlparser::ast::{
    BinaryOperator, Expr, Ident, ObjectName, OrderByExpr, Query, Select, SelectItem, SetExpr,
    Statement, Value as SqlValue,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::error::{QueryError, Result};
use crate::schema::ColumnName;

// ============================================================================
// Parsed AST Types
// ============================================================================

/// Parsed SELECT statement.
#[derive(Debug, Clone)]
pub struct ParsedSelect {
    /// Table name from FROM clause.
    pub table: String,
    /// Selected columns (None = SELECT *).
    pub columns: Option<Vec<ColumnName>>,
    /// WHERE predicates.
    pub predicates: Vec<Predicate>,
    /// ORDER BY clauses.
    pub order_by: Vec<OrderByClause>,
    /// LIMIT value.
    pub limit: Option<usize>,
}

/// A comparison predicate from the WHERE clause.
#[derive(Debug, Clone)]
pub enum Predicate {
    /// column = value or column = $N
    Eq(ColumnName, PredicateValue),
    /// column < value
    Lt(ColumnName, PredicateValue),
    /// column <= value
    Le(ColumnName, PredicateValue),
    /// column > value
    Gt(ColumnName, PredicateValue),
    /// column >= value
    Ge(ColumnName, PredicateValue),
    /// column IN (value, value, ...)
    In(ColumnName, Vec<PredicateValue>),
}

impl Predicate {
    /// Returns the column name this predicate operates on.
    #[allow(dead_code)]
    pub fn column(&self) -> &ColumnName {
        match self {
            Predicate::Eq(col, _)
            | Predicate::Lt(col, _)
            | Predicate::Le(col, _)
            | Predicate::Gt(col, _)
            | Predicate::Ge(col, _)
            | Predicate::In(col, _) => col,
        }
    }
}

/// A value in a predicate (literal or parameter reference).
#[derive(Debug, Clone)]
pub enum PredicateValue {
    /// Literal integer.
    Int(i64),
    /// Literal string.
    String(String),
    /// Literal boolean.
    Bool(bool),
    /// NULL literal.
    Null,
    /// Parameter placeholder ($1, $2, etc.) - 1-indexed.
    Param(usize),
}

/// ORDER BY clause.
#[derive(Debug, Clone)]
pub struct OrderByClause {
    /// Column to order by.
    pub column: ColumnName,
    /// Ascending (true) or descending (false).
    pub ascending: bool,
}

// ============================================================================
// Parser
// ============================================================================

/// Parses a SQL query string into a `ParsedSelect`.
pub fn parse_query(sql: &str) -> Result<ParsedSelect> {
    let dialect = GenericDialect {};
    let statements =
        Parser::parse_sql(&dialect, sql).map_err(|e| QueryError::ParseError(e.to_string()))?;

    if statements.len() != 1 {
        return Err(QueryError::ParseError(format!(
            "expected exactly 1 statement, got {}",
            statements.len()
        )));
    }

    match &statements[0] {
        Statement::Query(query) => parse_select_query(query),
        other => Err(QueryError::UnsupportedFeature(format!(
            "only SELECT queries are supported, got {other:?}"
        ))),
    }
}

fn parse_select_query(query: &Query) -> Result<ParsedSelect> {
    // Reject CTEs
    if query.with.is_some() {
        return Err(QueryError::UnsupportedFeature(
            "WITH clauses (CTEs) are not supported".to_string(),
        ));
    }

    let SetExpr::Select(select) = query.body.as_ref() else {
        return Err(QueryError::UnsupportedFeature(
            "only simple SELECT queries are supported".to_string(),
        ));
    };

    let parsed_select = parse_select(select)?;

    // Parse ORDER BY from query (not select)
    let order_by = match &query.order_by {
        Some(ob) => parse_order_by(ob)?,
        None => vec![],
    };

    // Parse LIMIT from query
    let limit = parse_limit(query.limit.as_ref())?;

    Ok(ParsedSelect {
        table: parsed_select.table,
        columns: parsed_select.columns,
        predicates: parsed_select.predicates,
        order_by,
        limit,
    })
}

fn parse_select(select: &Select) -> Result<ParsedSelect> {
    // Reject DISTINCT
    if select.distinct.is_some() {
        return Err(QueryError::UnsupportedFeature(
            "DISTINCT is not supported".to_string(),
        ));
    }

    // Parse FROM - must be exactly one table
    if select.from.len() != 1 {
        return Err(QueryError::ParseError(format!(
            "expected exactly 1 table in FROM clause, got {}",
            select.from.len()
        )));
    }

    let from = &select.from[0];

    // Reject JOINs
    if !from.joins.is_empty() {
        return Err(QueryError::UnsupportedFeature(
            "JOINs are not supported".to_string(),
        ));
    }

    let table = match &from.relation {
        sqlparser::ast::TableFactor::Table { name, .. } => object_name_to_string(name),
        other => {
            return Err(QueryError::UnsupportedFeature(format!(
                "unsupported FROM clause: {other:?}"
            )));
        }
    };

    // Parse SELECT columns
    let columns = parse_select_items(&select.projection)?;

    // Parse WHERE predicates
    let predicates = match &select.selection {
        Some(expr) => parse_where_expr(expr)?,
        None => vec![],
    };

    // Reject GROUP BY
    match &select.group_by {
        sqlparser::ast::GroupByExpr::Expressions(exprs, _) if !exprs.is_empty() => {
            return Err(QueryError::UnsupportedFeature(
                "GROUP BY is not supported".to_string(),
            ));
        }
        sqlparser::ast::GroupByExpr::All(_) => {
            return Err(QueryError::UnsupportedFeature(
                "GROUP BY ALL is not supported".to_string(),
            ));
        }
        sqlparser::ast::GroupByExpr::Expressions(_, _) => {}
    }

    // Reject HAVING
    if select.having.is_some() {
        return Err(QueryError::UnsupportedFeature(
            "HAVING is not supported".to_string(),
        ));
    }

    Ok(ParsedSelect {
        table,
        columns,
        predicates,
        order_by: vec![],
        limit: None,
    })
}

fn parse_select_items(items: &[SelectItem]) -> Result<Option<Vec<ColumnName>>> {
    let mut columns = Vec::new();

    for item in items {
        match item {
            SelectItem::Wildcard(_) => {
                // SELECT * - return None to indicate all columns
                return Ok(None);
            }
            SelectItem::UnnamedExpr(Expr::Identifier(ident)) => {
                columns.push(ColumnName::new(ident.value.clone()));
            }
            SelectItem::ExprWithAlias {
                expr: Expr::Identifier(ident),
                alias,
            } => {
                // For now, we ignore aliases and just use the column name
                let _ = alias;
                columns.push(ColumnName::new(ident.value.clone()));
            }
            other => {
                return Err(QueryError::UnsupportedFeature(format!(
                    "unsupported SELECT item: {other:?}"
                )));
            }
        }
    }

    Ok(Some(columns))
}

fn parse_where_expr(expr: &Expr) -> Result<Vec<Predicate>> {
    match expr {
        // AND combines multiple predicates
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut predicates = parse_where_expr(left)?;
            predicates.extend(parse_where_expr(right)?);
            Ok(predicates)
        }

        // Comparison operators
        Expr::BinaryOp { left, op, right } => {
            let predicate = parse_comparison(left, op, right)?;
            Ok(vec![predicate])
        }

        // IN list
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            if *negated {
                return Err(QueryError::UnsupportedFeature(
                    "NOT IN is not supported".to_string(),
                ));
            }

            let column = expr_to_column(expr)?;
            let values: Result<Vec<_>> = list.iter().map(expr_to_predicate_value).collect();
            Ok(vec![Predicate::In(column, values?)])
        }

        // Parenthesized expression
        Expr::Nested(inner) => parse_where_expr(inner),

        other => Err(QueryError::UnsupportedFeature(format!(
            "unsupported WHERE expression: {other:?}"
        ))),
    }
}

fn parse_comparison(left: &Expr, op: &BinaryOperator, right: &Expr) -> Result<Predicate> {
    let column = expr_to_column(left)?;
    let value = expr_to_predicate_value(right)?;

    match op {
        BinaryOperator::Eq => Ok(Predicate::Eq(column, value)),
        BinaryOperator::Lt => Ok(Predicate::Lt(column, value)),
        BinaryOperator::LtEq => Ok(Predicate::Le(column, value)),
        BinaryOperator::Gt => Ok(Predicate::Gt(column, value)),
        BinaryOperator::GtEq => Ok(Predicate::Ge(column, value)),
        other => Err(QueryError::UnsupportedFeature(format!(
            "unsupported operator: {other:?}"
        ))),
    }
}

fn expr_to_column(expr: &Expr) -> Result<ColumnName> {
    match expr {
        Expr::Identifier(ident) => Ok(ColumnName::new(ident.value.clone())),
        Expr::CompoundIdentifier(idents) if idents.len() == 2 => {
            // table.column - ignore table for now
            Ok(ColumnName::new(idents[1].value.clone()))
        }
        other => Err(QueryError::UnsupportedFeature(format!(
            "expected column name, got {other:?}"
        ))),
    }
}

fn expr_to_predicate_value(expr: &Expr) -> Result<PredicateValue> {
    match expr {
        Expr::Value(SqlValue::Number(n, _)) => {
            let v: i64 = n
                .parse()
                .map_err(|_| QueryError::ParseError(format!("invalid integer: {n}")))?;
            Ok(PredicateValue::Int(v))
        }
        Expr::Value(SqlValue::SingleQuotedString(s) | SqlValue::DoubleQuotedString(s)) => {
            Ok(PredicateValue::String(s.clone()))
        }
        Expr::Value(SqlValue::Boolean(b)) => Ok(PredicateValue::Bool(*b)),
        Expr::Value(SqlValue::Null) => Ok(PredicateValue::Null),
        Expr::Value(SqlValue::Placeholder(p)) => {
            // Parse $1, $2, etc.
            if let Some(num_str) = p.strip_prefix('$') {
                let idx: usize = num_str.parse().map_err(|_| {
                    QueryError::ParseError(format!("invalid parameter placeholder: {p}"))
                })?;
                Ok(PredicateValue::Param(idx))
            } else {
                Err(QueryError::ParseError(format!(
                    "unsupported placeholder format: {p}"
                )))
            }
        }
        Expr::UnaryOp {
            op: sqlparser::ast::UnaryOperator::Minus,
            expr,
        } => {
            // Handle negative numbers
            if let Expr::Value(SqlValue::Number(n, _)) = expr.as_ref() {
                let v: i64 = n
                    .parse::<i64>()
                    .map_err(|_| QueryError::ParseError(format!("invalid integer: -{n}")))?;
                Ok(PredicateValue::Int(-v))
            } else {
                Err(QueryError::UnsupportedFeature(format!(
                    "unsupported unary minus operand: {expr:?}"
                )))
            }
        }
        other => Err(QueryError::UnsupportedFeature(format!(
            "unsupported value expression: {other:?}"
        ))),
    }
}

fn parse_order_by(order_by: &sqlparser::ast::OrderBy) -> Result<Vec<OrderByClause>> {
    let mut clauses = Vec::new();

    for expr in &order_by.exprs {
        clauses.push(parse_order_by_expr(expr)?);
    }

    Ok(clauses)
}

fn parse_order_by_expr(expr: &OrderByExpr) -> Result<OrderByClause> {
    let column = match &expr.expr {
        Expr::Identifier(ident) => ColumnName::new(ident.value.clone()),
        other => {
            return Err(QueryError::UnsupportedFeature(format!(
                "unsupported ORDER BY expression: {other:?}"
            )));
        }
    };

    let ascending = expr.asc.unwrap_or(true);

    Ok(OrderByClause { column, ascending })
}

fn parse_limit(limit: Option<&Expr>) -> Result<Option<usize>> {
    match limit {
        None => Ok(None),
        Some(Expr::Value(SqlValue::Number(n, _))) => {
            let v: usize = n
                .parse()
                .map_err(|_| QueryError::ParseError(format!("invalid LIMIT value: {n}")))?;
            Ok(Some(v))
        }
        Some(other) => Err(QueryError::UnsupportedFeature(format!(
            "unsupported LIMIT expression: {other:?}"
        ))),
    }
}

fn object_name_to_string(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|i: &Ident| i.value.clone())
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_select() {
        let result = parse_query("SELECT id, name FROM users").unwrap();
        assert_eq!(result.table, "users");
        assert_eq!(
            result.columns,
            Some(vec![ColumnName::new("id"), ColumnName::new("name")])
        );
        assert!(result.predicates.is_empty());
    }

    #[test]
    fn test_parse_select_star() {
        let result = parse_query("SELECT * FROM users").unwrap();
        assert_eq!(result.table, "users");
        assert!(result.columns.is_none());
    }

    #[test]
    fn test_parse_where_eq() {
        let result = parse_query("SELECT * FROM users WHERE id = 42").unwrap();
        assert_eq!(result.predicates.len(), 1);
        match &result.predicates[0] {
            Predicate::Eq(col, PredicateValue::Int(42)) => {
                assert_eq!(col.as_str(), "id");
            }
            other => panic!("unexpected predicate: {other:?}"),
        }
    }

    #[test]
    fn test_parse_where_string() {
        let result = parse_query("SELECT * FROM users WHERE name = 'alice'").unwrap();
        match &result.predicates[0] {
            Predicate::Eq(col, PredicateValue::String(s)) => {
                assert_eq!(col.as_str(), "name");
                assert_eq!(s, "alice");
            }
            other => panic!("unexpected predicate: {other:?}"),
        }
    }

    #[test]
    fn test_parse_where_and() {
        let result = parse_query("SELECT * FROM users WHERE id = 1 AND name = 'bob'").unwrap();
        assert_eq!(result.predicates.len(), 2);
    }

    #[test]
    fn test_parse_where_in() {
        let result = parse_query("SELECT * FROM users WHERE id IN (1, 2, 3)").unwrap();
        match &result.predicates[0] {
            Predicate::In(col, values) => {
                assert_eq!(col.as_str(), "id");
                assert_eq!(values.len(), 3);
            }
            other => panic!("unexpected predicate: {other:?}"),
        }
    }

    #[test]
    fn test_parse_order_by() {
        let result = parse_query("SELECT * FROM users ORDER BY name ASC, id DESC").unwrap();
        assert_eq!(result.order_by.len(), 2);
        assert_eq!(result.order_by[0].column.as_str(), "name");
        assert!(result.order_by[0].ascending);
        assert_eq!(result.order_by[1].column.as_str(), "id");
        assert!(!result.order_by[1].ascending);
    }

    #[test]
    fn test_parse_limit() {
        let result = parse_query("SELECT * FROM users LIMIT 10").unwrap();
        assert_eq!(result.limit, Some(10));
    }

    #[test]
    fn test_parse_param() {
        let result = parse_query("SELECT * FROM users WHERE id = $1").unwrap();
        match &result.predicates[0] {
            Predicate::Eq(_, PredicateValue::Param(1)) => {}
            other => panic!("unexpected predicate: {other:?}"),
        }
    }

    #[test]
    fn test_reject_join() {
        let result = parse_query("SELECT * FROM users JOIN orders ON users.id = orders.user_id");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_subquery() {
        let result = parse_query("SELECT * FROM (SELECT * FROM users)");
        assert!(result.is_err());
    }
}
