use std::{
    cell::RefCell,
    collections::{hash_map::DefaultHasher, HashMap},
    fmt::Display,
    hash::{Hash, Hasher},
    rc::Rc,
};

use crate::ast::{
    Binary, Call, Element, First, Function, If, Let, Location, Print, Second, Term, Var,
};

#[derive(Clone, Debug)]
pub struct Closure {
    parameters: Vec<Var>,
    body: Box<Term>,
    context: Rc<RefCell<Context>>,
}

#[derive(Clone, Debug)]
pub struct Tuple {
    first: Box<Value>,
    second: Box<Value>,
}

impl Display for Tuple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let first = self.first.clone();
        let second = self.second.clone();

        write!(f, "({first}, {second})")
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Closure(Closure),
    Int(i64),
    Str(String),
    Bool(bool),
    Tuple(Tuple),
}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Closure(_closure) => panic!("this should never be executed"),
            Self::Int(int) => format!("Int({int})").hash(state),
            Self::Str(string) => format!("Str({string})").hash(state),
            Self::Bool(bool) => format!("Bool({bool})").hash(state),
            Self::Tuple(tuple) => format!("Tuple({tuple})").hash(state),
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Closure(_closure) => String::from("[closure]"),
            Self::Int(int) => int.to_string(),
            Self::Str(str) => str.to_string(),
            Self::Bool(bool) => bool.to_string(),
            Self::Tuple(tuple) => {
                format!(
                    "({}, {})",
                    tuple.first.to_string(),
                    tuple.second.to_string()
                )
            }
        };

        f.write_str(&value)
    }
}

pub type Cache = std::collections::HashMap<String, Value>;
pub type Context = HashMap<String, Value>;

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub message: String,
    pub full_text: String,
    pub location: Location,
}

fn eval_let<I: Printer>(
    let_: Let,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    let name = let_.name.text;

    match eval(let_.value, context, cache, io)? {
        Value::Closure(closure) => {
            let self_ = Value::Closure(Closure {
                parameters: closure.parameters,
                body: closure.body,
                context: closure.context.clone(),
            });

            closure
                .context
                .borrow_mut()
                .insert(name.clone(), self_.clone());

            context.insert(name, self_.clone());
        }
        value => {
            context.insert(name, value);
        }
    }

    eval(let_.next, context, cache, io)
}

fn cache_key(body: &Box<Term>, arguments: Vec<Value>) -> Option<String> {
    let arguments = arguments
        .into_iter()
        .map(|argument| match argument {
            Value::Closure(_) => None,
            value => {
                let mut s = DefaultHasher::new();
                // TODO: is ok to define the hasher on each iteration?
                value.hash(&mut s);
                Some(s.finish().to_string())
            }
        })
        .collect::<Option<Vec<String>>>()?;

    let mut s = DefaultHasher::new();
    (*body.clone(), arguments).hash(&mut s);

    Some(s.finish().to_string())
}

fn eval_memo<I: Printer>(
    body: Box<Term>,
    arguments: Vec<Value>,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    match cache_key(&body, arguments.clone()) {
        Some(cache_key) => match cache.get(&cache_key) {
            Some(cached_value) => Ok(cached_value.clone()),
            None => {
                let value = eval(body, context, cache, io)?;
                cache.insert(cache_key, value.clone());

                Ok(value)
            }
        },
        None => eval(body, context, cache, io),
    }
}

fn eval_call<I: Printer>(
    call: Call,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    match eval(call.callee, context, cache, io)? {
        Value::Closure(closure) => {
            let mut new_context = closure.context.borrow_mut().clone();
            let mut arguments = Vec::new();

            for (parameter, argument) in closure.parameters.clone().into_iter().zip(call.arguments)
            {
                let argument = eval(Box::new(argument), context, cache, io)?;
                arguments.push(argument.clone());

                new_context.insert(parameter.text, argument);
            }

            match closure.body.is_pure() {
                true => eval_memo(closure.body, arguments, &mut new_context, cache, io),
                false => eval(closure.body, &mut new_context, cache, io),
            }
        }
        value => Err(RuntimeError {
            message: String::from("invalid function call"),
            full_text: format!("{} cannot be called as a function", value),
            location: call.location,
        }),
    }
}

fn eval_if<I: Printer>(
    if_: If,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    let condition_result = eval(if_.condition.clone(), context, cache, io)?;
    let condition = match condition_result {
        Value::Bool(bool) => Ok(bool),
        _ => Err(RuntimeError {
            message: String::from("invalid if condition"),
            full_text: format!(
                "{} can't be used as an if condition. use a boolean instead",
                condition_result
            ),
            location: if_.condition.location().clone(),
        }),
    }?;

    match condition {
        true => eval(if_.then, context, cache, io),
        false => eval(if_.otherwise, context, cache, io),
    }
}

fn eval_binary<I: Printer>(
    binary: Binary,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    let lhs = eval(binary.lhs.clone(), context, cache, io)?;
    let rhs = eval(binary.rhs.clone(), context, cache, io)?;

    lhs.binary_op(binary, rhs)
}

fn eval_var(var: Var, context: &mut Context) -> Result<Value, RuntimeError> {
    context
        .get(&var.text)
        .ok_or(RuntimeError {
            message: format!("unbound variable \"{}\"", var.text),
            full_text: format!(
                "variable \"{}\" was not defined in the current scope",
                var.text
            ),
            location: var.location,
        })
        .map(|value| value.clone())
}

fn eval_tuple<I: Printer>(
    tuple: crate::ast::Tuple,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    let first = eval(tuple.first, context, cache, io)?;
    let second = eval(tuple.second, context, cache, io)?;

    Ok(Value::Tuple(Tuple {
        first: Box::new(first),
        second: Box::new(second),
    }))
}

fn eval_first<I: Printer>(
    first: First,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    match eval(first.value, context, cache, io)? {
        Value::Tuple(Tuple { first, second: _ }) => Ok(*first),
        _value => Err(RuntimeError {
            message: String::from("invalid expression"),
            full_text: String::from("cannot use first operation from anything but a tuple"),
            location: first.location,
        }),
    }
}

fn eval_second<I: Printer>(
    second: Second,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    match eval(second.value, context, cache, io)? {
        Value::Tuple(Tuple { first: _, second }) => Ok(*second),
        _value => Err(RuntimeError {
            message: String::from("invalid expression"),
            full_text: String::from("cannot use second operation from anything but a tuple"),
            location: second.location,
        }),
    }
}

pub struct IO;

pub trait Printer {
    fn print(&mut self, value: Value) -> Value;
}
impl Printer for IO {
    fn print(&mut self, value: Value) -> Value {
        println!("{}", &value);

        value
    }
}

fn eval_print<I: Printer>(
    print_: Print,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    let value = eval(print_.value, context, cache, io)?;

    Ok(io.print(value))
}

fn eval_function(function: Function, context: &mut Context) -> Result<Value, RuntimeError> {
    let context = Rc::new(RefCell::new(context.clone()));

    Ok(Value::Closure(Closure {
        parameters: function.parameters,
        body: function.value.clone(),
        context,
    }))
}

pub fn eval<I: Printer>(
    term: Box<Term>,
    context: &mut Context,
    cache: &mut Cache,
    io: &mut I,
) -> Result<Value, RuntimeError> {
    match *term {
        Term::Let(let_) => eval_let(let_, context, cache, io),
        Term::Int(int) => Ok(Value::Int(int.value)),
        Term::Str(str) => Ok(Value::Str(str.value)),
        Term::Bool(bool) => Ok(Value::Bool(bool.value)),
        Term::Function(function) => eval_function(function, context),
        Term::Call(call) => eval_call(call, context, cache, io),
        Term::If(if_) => eval_if(if_, context, cache, io),
        Term::Binary(binary) => eval_binary(binary, context, cache, io),
        Term::Var(var) => eval_var(var, context),
        Term::Tuple(tuple) => eval_tuple(tuple, context, cache, io),
        Term::First(first) => eval_first(first, context, cache, io),
        Term::Second(second) => eval_second(second, context, cache, io),
        Term::Print(print) => eval_print(print, context, cache, io),
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{Location, Term, Tuple, Var};

    use super::{eval, Cache, Context, Printer, Value};

    #[derive(Default)]
    struct DummyIO(String);

    impl Printer for DummyIO {
        fn print(&mut self, value: super::Value) -> super::Value {
            self.0.push_str(&format!("{}\n", value));

            value
        }
    }

    fn location() -> Location {
        Location {
            start: 0,
            end: 0,
            filename: "tests".to_string(),
        }
    }

    fn int(int: i64) -> Term {
        Term::Int(crate::ast::Int {
            value: int,
            location: location(),
        })
    }

    fn v_int(int: i64) -> Value {
        Value::Int(int)
    }

    fn v_tuple(first: Value, second: Value) -> Value {
        Value::Tuple(super::Tuple {
            first: Box::new(first),
            second: Box::new(second),
        })
    }

    fn var(str: &str) -> Var {
        Var {
            text: str.to_string(),
            location: location(),
        }
    }

    fn let_(name: &str, value: Term, next: Term) -> Term {
        Term::Let(crate::ast::Let {
            name: var(name),
            value: Box::new(value),
            next: Box::new(next),
            location: location(),
        })
    }

    fn print_(value: Term) -> Term {
        Term::Print(crate::ast::Print {
            value: Box::new(value),
            location: location(),
        })
    }

    fn tuple(first: Term, second: Term) -> Term {
        Term::Tuple(Tuple {
            first: Box::new(first),
            second: Box::new(second),
            location: location(),
        })
    }

    fn add(lhs: Term, rhs: Term) -> Term {
        Term::Binary(super::Binary {
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            op: crate::ast::BinaryOp::Add,
            location: location(),
        })
    }

    fn var_(text: &str) -> Term {
        Term::Var(Var {
            text: text.to_string(),
            location: location(),
        })
    }

    fn eq(l: Value, r: Value) -> bool {
        match l.eq(&r, &location()).unwrap() {
            Value::Bool(bool) => bool,
            _ => panic!(),
        }
    }

    #[test]
    fn print_inner_and_outer_scope() {
        let mut io = DummyIO::default();

        let let_ = let_("_", print_(int(1)), print_(int(2)));
        let mut context = Context::new();
        let mut cache = Cache::new();
        let result = eval(Box::new(let_), &mut context, &mut cache, &mut io).unwrap();

        assert!(eq(result, v_int(2)));
        assert_eq!(io.0, "1\n2\n");
    }

    #[test]
    fn print_inside_var_scope_and_var() {
        let mut io = DummyIO::default();

        let let_ = let_(
            "tuple",
            tuple(print_(int(1)), print_(int(2))),
            print_(var_("tuple")),
        );
        let mut context = Context::new();
        let mut cache = Cache::new();
        let result = eval(Box::new(let_), &mut context, &mut cache, &mut io).unwrap();

        assert_eq!(result.to_string(), v_tuple(v_int(1), v_int(2)).to_string());
        assert_eq!(io.0, "1\n2\n(1, 2)\n");
    }

    #[test]
    fn print_sum_operation_and_operated() {
        let mut io = DummyIO::default();

        let print = print_(add(print_(int(1)), print_(int(2))));
        let mut context = Context::new();
        let mut cache = Cache::new();
        let result = eval(Box::new(print), &mut context, &mut cache, &mut io).unwrap();

        assert!(eq(result, v_int(3)));
        assert_eq!(io.0, "1\n2\n3\n");
    }
}
