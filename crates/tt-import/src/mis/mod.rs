//! The parser of the mission files (`.mis`, a script text of object
//! declarations): tokens ([`lexer`]), the statement tree ([`ast`]) and
//! [`parser::parse`]. The importer converts what it reads to the mission's
//! RON file ([`crate::convert`]).

pub mod ast;
pub mod lexer;
pub mod parser;
