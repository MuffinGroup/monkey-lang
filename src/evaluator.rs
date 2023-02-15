use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{
    BlockStatement, Expression, HashLiteral, Identifier, IfExpression, Node, Program, Statement,
};
use crate::builtin::Builtin;
use crate::enviroment::Enviroment;
use crate::object::{
    Array, Boolean, BuiltinFunction, Function, Hash, HashKeyable, Inspector, Integer, Null, Object,
    ReturnValue, RuntimeError, Str,
};

pub struct Evaluator<'a> {
    env: Rc<RefCell<Enviroment>>,
    builtin: Builtin<'a>,
}

impl<'a> Evaluator<'a> {
    pub fn new(env: Enviroment) -> Self {
        Self {
            env: Rc::new(RefCell::new(env)),
            builtin: Builtin::new(),
        }
    }

    pub fn eval(&mut self, ast: Node) -> Object {
        match ast {
            Node::Program(node) => self.eval_program(node),
            // Node::Statement(node) => eval(ast)
            Node::Statement(Statement::Expression(node)) => {
                self.eval(Node::Expression(&node.expression))
            }

            Node::Statement(Statement::Return(node)) => {
                let value = self.eval(Node::Expression(&node.return_value));
                if self.is_error(&value) {
                    value
                } else {
                    Object::ReturnValue(ReturnValue::new(value))
                }
            }

            Node::Statement(Statement::Let(node)) => {
                let value = self.eval(Node::Expression(&node.value));
                if self.is_error(&value) {
                    value
                } else {
                    self.env
                        .borrow_mut()
                        .set(node.name.value.to_owned(), value.clone());
                    value
                }
            }

            Node::BlockStatement(node) => self.eval_block_statement(node),

            Node::Expression(Expression::Identifier(node)) => self.eval_indentifier(node),

            Node::Expression(Expression::IntegerLiteral(node)) => {
                Object::Integer(Integer::new(node.value))
            }

            Node::Expression(Expression::StringLiteral(node)) => {
                Object::Str(Str::new(node.value.to_owned()))
            }

            Node::Expression(Expression::ArrayLiteral(node)) => {
                let elements = self.eval_expressions(node.elements.clone());
                if elements.len() == 1 && self.is_error(&elements[0]) {
                    return elements[0].clone();
                }
                Object::Array(Array::new(elements))
            }

            Node::Expression(Expression::HashLiteral(node)) => self.eval_hash_literal(node),

            Node::Expression(Expression::Index(node)) => {
                let left = self.eval(Node::Expression(&node.left));
                if self.is_error(&left) {
                    return left;
                }
                let index = self.eval(Node::Expression(&node.index));
                if self.is_error(&index) {
                    return index;
                }
                self.eval_index_expression(left, index)
            }

            Node::Expression(Expression::Boolean(node)) => {
                self.native_bool_to_boolean_object(node.value)
            }

            Node::Expression(Expression::FunctionLiteral(node)) => Object::Function(Function::new(
                node.parameters.to_owned(),
                node.body.to_owned(),
                Rc::clone(&self.env),
            )),

            Node::Expression(Expression::Prefix(node)) => {
                let right = self.eval(Node::Expression(node.right.as_ref()));
                if self.is_error(&right) {
                    right
                } else {
                    self.eval_prefix_expression(node.operator.to_owned(), right)
                }
            }

            Node::Expression(Expression::Infix(node)) => {
                let left = self.eval(Node::Expression(node.left.as_ref()));

                if self.is_error(&left) {
                    return left;
                }

                let right = self.eval(Node::Expression(node.right.as_ref()));

                if self.is_error(&right) {
                    return right;
                }

                self.eval_infix_expression(node.operator.to_owned(), left, right)
            }

            Node::Expression(Expression::If(node)) => self.eval_if_expression(node),

            Node::Expression(Expression::Call(node)) => {
                let func = self.eval(Node::Expression(&node.function));

                if self.is_error(&func) {
                    return func;
                }

                match func {
                    Object::Function(func) => {
                        let args = self.eval_expressions(node.arguments.clone());
                        if args.len() == 1 && self.is_error(&args[0]) {
                            return args[0].clone();
                        }
                        self.apply_function(&func, args)
                    }
                    Object::BuiltinFunction(func) => {
                        let args = self.eval_expressions(node.arguments.clone());
                        if args.len() == 1 && self.is_error(&args[0]) {
                            return args[0].clone();
                        }
                        if let Some(result) = self.builtin.try_function(&func.name, args) {
                            result
                        } else {
                            Object::RuntimeError(RuntimeError::new(format!(
                                "Not a function: {}",
                                func.name,
                            )))
                        }
                    }
                    _ => Object::RuntimeError(RuntimeError::new(format!(
                        "Not a function: {}",
                        func.kind(),
                    ))),
                }
            }
        }
    }

    fn apply_function(&mut self, func: &Function, args: Vec<Object>) -> Object {
        let old_env = Rc::clone(&self.env);
        self.extend_function_env(func, args);
        let return_value = self.eval(Node::BlockStatement(&func.body));
        self.env = old_env;
        self.unwrap_return_value(return_value)
    }

    fn extend_function_env(&mut self, func: &Function, args: Vec<Object>) {
        let mut env = Enviroment::new(Some(Rc::clone(&func.env)));
        for (i, param) in func.parameters.iter().enumerate() {
            env.set(param.value.to_owned(), args[i].to_owned());
        }
        self.env = Rc::new(RefCell::new(env));
    }

    fn unwrap_return_value(&self, obj: Object) -> Object {
        if let Object::ReturnValue(return_value) = obj {
            *return_value.value
        } else {
            obj
        }
    }

    fn eval_program(&mut self, program: &Program) -> Object {
        let mut result = Object::Null(Null {});

        for stmt in program.statements.iter() {
            result = self.eval(Node::Statement(stmt));

            match result {
                Object::ReturnValue(return_object) => return *return_object.value,
                Object::RuntimeError(_) => return result,
                _ => {}
            }
        }

        result
    }

    fn eval_expressions(&mut self, exps: Vec<Expression>) -> Vec<Object> {
        let mut result: Vec<_> = vec![];

        for exp in exps.iter() {
            let evaluated = self.eval(Node::Expression(exp));
            if self.is_error(&evaluated) {
                return vec![evaluated];
            }
            result.push(evaluated);
        }

        result
    }

    fn eval_block_statement(&mut self, block: &BlockStatement) -> Object {
        let mut result = Object::Null(Null {});

        for stmt in block.statements.iter() {
            result = self.eval(Node::Statement(stmt));

            match result {
                Object::ReturnValue(_) => return result,
                Object::RuntimeError(_) => return result,
                _ => {}
            }
        }

        result
    }

    fn eval_index_expression(&mut self, left: Object, index: Object) -> Object {
        match left {
            Object::Array(array) => match index {
                Object::Integer(index) => {
                    if index.value < 0
                        || array.elements.len() < (index.value + 1).try_into().unwrap()
                    {
                        Object::Null(Null::default())
                    } else {
                        array.elements[index.value as usize].clone()
                    }
                }
                _ => Object::RuntimeError(RuntimeError::new(format!(
                    "index is not a integer: {}",
                    index.kind()
                ))),
            },
            _ => Object::RuntimeError(RuntimeError::new(format!(
                "index operator not supported: {}",
                left.kind()
            ))),
        }
    }

    fn eval_prefix_expression(&mut self, operator: String, right: Object) -> Object {
        match operator.as_str() {
            "!" => self.eval_bang_operator_expression(right),
            "-" => self.eval_minux_operator_expression(right),
            _ => Object::RuntimeError(RuntimeError::new(format!(
                "unknown operator: {}{}",
                operator,
                right.kind()
            ))),
        }
    }

    fn eval_infix_expression(&self, operator: String, left: Object, right: Object) -> Object {
        if left.kind() != right.kind() {
            return Object::RuntimeError(RuntimeError::new(format!(
                "type mismatch: {} {} {}",
                left.kind(),
                operator,
                right.kind()
            )));
        }

        if let Object::Integer(ref left) = left {
            if let Object::Integer(ref right) = right {
                return self.eval_integer_infix_expression(operator, left, right);
            }
        }

        if let Object::Str(ref left) = left {
            if let Object::Str(ref right) = right {
                return self.eval_string_infix_expression(operator, left, right);
            }
        }

        match operator.as_str() {
            "==" => {
                let result = left == right;
                self.native_bool_to_boolean_object(result)
            }
            "!=" => {
                let result = left != right;
                self.native_bool_to_boolean_object(result)
            }
            _ => Object::RuntimeError(RuntimeError::new(format!(
                "unknown operator: {} {} {}",
                left.kind(),
                operator,
                right.kind()
            ))),
        }
    }

    fn eval_string_infix_expression(&self, operator: String, left: &Str, right: &Str) -> Object {
        match operator.as_str() {
            "+" => Object::Str(Str::new(format!("{}{}", left.value, right.value))),
            _ => Object::RuntimeError(RuntimeError::new(format!(
                "unknown operator: {} {} {}",
                left.kind(),
                operator,
                right.kind(),
            ))),
        }
    }

    fn eval_integer_infix_expression(
        &self,
        operator: String,
        left: &Integer,
        right: &Integer,
    ) -> Object {
        let left_value = left.value;
        let right_value = right.value;

        match operator.as_str() {
            "+" => Object::Integer(Integer::new(left_value + right_value)),
            "-" => Object::Integer(Integer::new(left_value - right_value)),
            "*" => Object::Integer(Integer::new(left_value * right_value)),
            "/" => Object::Integer(Integer::new(left_value / right_value)),
            "<" => self.native_bool_to_boolean_object(left_value < right_value),
            ">" => self.native_bool_to_boolean_object(left_value > right_value),
            "==" => self.native_bool_to_boolean_object(left_value == right_value),
            "!=" => self.native_bool_to_boolean_object(left_value != right_value),
            _ => Object::RuntimeError(RuntimeError::new(format!(
                "unknown operator: {} {} {}",
                left.kind(),
                operator,
                right.kind()
            ))),
        }
    }

    fn eval_hash_literal(&mut self, node: &HashLiteral) -> Object {
        let mut hash_value = HashMap::new();

        for member in node.members.iter() {
            let key = self.eval(Node::Expression(&member.key));

            if self.is_error(&key) {
                return key;
            }

            let hash_key = match key {
                Object::Str(o) => o.hash_key(),
                Object::Integer(o) => o.hash_key(),
                Object::Boolean(o) => o.hash_key(),
                _ => return Object::RuntimeError(RuntimeError::new(format!(
                    "only string, integer and boolean can be hash key, found {}",
                    key.kind()
                ))),
            };

            let value = self.eval(Node::Expression(&member.value));

            if self.is_error(&value) {
                return value;
            }
            hash_value.insert(hash_key.stringify(), value);
        }

        Object::Hash(Hash::new(hash_value))
    }

    fn eval_bang_operator_expression(&self, right: Object) -> Object {
        match right {
            Object::Boolean(object) => {
                if object.value {
                    Object::Boolean(Boolean::new(false))
                } else {
                    Object::Boolean(Boolean::new(true))
                }
            }
            Object::Null(_) => Object::Null(Null {}),
            _ => Object::Boolean(Boolean::new(false)),
        }
    }

    fn eval_minux_operator_expression(&self, right: Object) -> Object {
        match right {
            Object::Integer(object) => Object::Integer(Integer::new(-object.value)),
            _ => Object::RuntimeError(RuntimeError::new(format!(
                "unknown operator: -{}",
                right.kind()
            ))),
        }
    }

    fn eval_if_expression(&mut self, ie: &IfExpression) -> Object {
        let condition = self.eval(Node::Expression(&ie.condition));

        if self.is_error(&condition) {
            return condition;
        }

        if self.is_truthy(condition) {
            return self.eval(Node::BlockStatement(&ie.consequence));
        } else if let Some(ref alternative) = ie.alternative {
            return self.eval(Node::BlockStatement(alternative));
        }

        Object::Null(Null {})
    }

    fn eval_indentifier(&self, node: &Identifier) -> Object {
        if let Some(value) = self.env.borrow().get(node.value.to_owned()) {
            value
        } else if self.builtin.function_exists(&node.value) {
            Object::BuiltinFunction(BuiltinFunction::new(node.value.to_owned()))
        } else {
            Object::RuntimeError(RuntimeError::new(format!(
                "identifier not found: {}",
                node.value
            )))
        }
    }

    fn native_bool_to_boolean_object(&self, input: bool) -> Object {
        if input {
            Object::Boolean(Boolean::new(true))
        } else {
            Object::Boolean(Boolean::new(false))
        }
    }

    fn is_truthy(&self, object: Object) -> bool {
        match object {
            Object::Null(_) => false,
            Object::Boolean(obj) => obj.value,
            _ => true,
        }
    }

    fn is_error(&self, object: &Object) -> bool {
        matches!(object, Object::RuntimeError(_))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Evaluator;
    use crate::{
        ast::Node,
        enviroment::Enviroment,
        lexer::Lexer,
        object::{HashKeyable, Inspector, Integer, Object, Str, Boolean},
        parser::Parser,
    };

    fn test_eval(input: &str) -> Object {
        let mut lexer = Lexer::new(input.into());
        let mut parser = Parser::new(&mut lexer);
        let env = Enviroment::default();
        let program = parser.parse_program();
        let mut evaluator = Evaluator::new(env);
        evaluator.eval(Node::Program(&program))
    }

    fn test_integer_object(object: Object, expected: i64) {
        if let Object::Integer(integer) = object {
            assert_eq!(integer.value, expected);
        } else {
            panic!("not a integer");
        }
    }

    fn test_string_object(object: Object, expected: String) {
        if let Object::Str(string) = object {
            assert_eq!(string.value, expected);
        } else {
            panic!("not a string");
        }
    }

    fn test_boolean_object(object: Object, expected: bool) {
        if let Object::Boolean(bool) = object {
            assert_eq!(bool.value, expected);
        } else {
            panic!("not a boolean");
        }
    }

    fn test_null_object(object: Object) {
        assert!(matches!(object, Object::Null(_)))
    }

    fn test_error_object(object: Object, expected: String) {
        if let Object::RuntimeError(error) = object {
            assert_eq!(error.inspect(), format!("Error: {}", expected));
        } else {
            panic!("not a error")
        }
    }

    #[test]
    fn test_eval_integer_expressions() {
        let tests = [
            ("5", 5),
            ("10", 10),
            ("-5", -5),
            ("-10", -10),
            ("5 + 5 + 5 + 5 - 10", 10),
            ("2 * 2 * 2 * 2 * 2", 32),
            ("-50 + 100 + -50", 0),
            ("5 * 2 + 10", 20),
            ("5 + 2 * 10", 25),
            ("20 + 2 * -10", 0),
            ("50 / 2 * 2 + 10", 60),
            ("2 * (5 + 10)", 30),
            ("3 * 3 * 3 + 10", 37),
            ("3 * (3 * 3) + 10", 37),
            ("(5 + 10 * 2 + 15 / 3) * 2 + -10", 50),
        ];

        for (input, output) in tests.iter() {
            test_integer_object(test_eval(input), *output);
        }
    }

    #[test]
    fn test_eval_boolean_expresions() {
        let tests = [
            ("true", true),
            ("false", false),
            ("1 < 2", true),
            ("1 > 2", false),
            ("1 < 1", false),
            ("1 > 1", false),
            ("1 == 1", true),
            ("1 != 1", false),
            ("1 == 2", false),
            ("1 != 2", true),
            ("true == true", true),
            ("false == false", true),
            ("true == false", false),
            ("true != false", true),
            ("false != true", true),
            ("(1 < 2) == true", true),
            ("(1 < 2) == false", false),
            ("(1 > 2) == true", false),
            ("(1 > 2) == false", true),
        ];

        for (input, output) in tests.iter() {
            test_boolean_object(test_eval(input), *output);
        }
    }

    #[test]
    fn test_eval_bang_operators() {
        let tests = [
            ("!true", false),
            ("!false", true),
            ("!5", false),
            ("!!true", true),
            ("!!false", false),
            ("!!5", true),
        ];

        for (input, output) in tests.iter() {
            test_boolean_object(test_eval(input), *output);
        }
    }

    #[test]
    fn test_if_else_expressions() {
        let tests = [
            ("if (true) { 10 )", Some(10)),
            ("if (false) { 10 )", None),
            ("if (1) { 10 )", Some(10)),
            ("if (1 < 2) { 10 }", Some(10)),
            ("if (1 > 2) { 10 }", None),
            ("if (1 > 2) { 10 } else { 20 }", Some(20)),
            ("if (1 < 2) { 10 } else { 20 }", Some(10)),
        ];

        for (input, output) in tests.iter() {
            let evaluated = test_eval(input);
            if let Some(integer) = output {
                test_integer_object(evaluated, *integer as i64);
            } else {
                test_null_object(evaluated);
            }
        }
    }

    #[test]
    fn test_return_statements() {
        let tests = [
            ("return 10;", 10),
            ("return 10; 9;", 10),
            ("return 2 * 5; 9;", 10),
            ("9; return 2 * 5; 9;", 10),
            (
                "if (10 > 1) {
                    if (10 > 1) {
                        return 10;
                    }
                    return 1;
                }",
                10,
            ),
        ];

        for (input, output) in tests.iter() {
            println!("{} = {}", input, output);
            test_integer_object(test_eval(input), *output);
        }
    }

    #[test]
    fn test_error_handling() {
        let tests = [
            ("5 + true;", "type mismatch: INTEGER + BOOLEAN"),
            ("5 + true; 5;", "type mismatch: INTEGER + BOOLEAN"),
            ("-true", "unknown operator: -BOOLEAN"),
            ("true + false;", "unknown operator: BOOLEAN + BOOLEAN"),
            (
                "if (10 > 1) { true + false; }",
                "unknown operator: BOOLEAN + BOOLEAN",
            ),
            (
                "if (10 > 1) {
                  if (10 > 1) {
                    return true + false;
                  }
                  return 1;
                }",
                "unknown operator: BOOLEAN + BOOLEAN",
            ),
            ("foobar", "identifier not found: foobar"),
            ("\"Hello\" - \"World\"", "unknown operator: STRING - STRING"),
        ];

        for (input, output) in tests.iter() {
            println!("{} => {}", input, output);
            let evaluated = test_eval(input);
            test_error_object(evaluated, output.to_string());
        }
    }

    #[test]
    fn test_let_statements() {
        let tests = [
            ("let a = 5; a;", 5),
            ("let a = 5 * 5; a;", 25),
            ("let a = 5; let b = a; b;", 5),
            ("let a = 5; let b = a; let c = a + b + 5; c;", 15),
        ];

        for (input, output) in tests.iter() {
            test_integer_object(test_eval(input), *output);
        }
    }

    #[test]
    fn test_function_object() {
        let input = "fn(x) { x + 2 };";
        let evaluated = test_eval(input);

        if let Object::Function(object) = evaluated {
            assert_eq!(object.parameters.len(), 1);
            assert_eq!(object.parameters[0].value, "x".to_string());
            assert_eq!(format!("{}", object.body), "(x + 2)");
        } else {
            panic!("not a function")
        }
    }

    #[test]
    fn test_function_application() {
        let tests = [
            ("let identity = fn(x) { x; }; identity(5);", 5),
            ("let identity = fn(x) { return x; }; identity(5);", 5),
            ("let double = fn(x) { x * 2; }; double(5);", 10),
            ("let add = fn(x, y) { x + y; }; add(5, 5);", 10),
            ("let add = fn(x, y) { x + y; }; add(5 + 5, add(5, 5));", 20),
            ("fn(x) { x; }(5)", 5),
        ];

        for (input, output) in tests.iter() {
            test_integer_object(test_eval(input), *output);
        }
    }

    #[test]
    fn test_closures() {
        let input = "
            let newAdder = fn(x) {
                fn(y) { x + y };
            };
            let addTwo = newAdder(2);
            addTwo(2);
        ";

        test_integer_object(test_eval(input), 4);
    }

    #[test]
    fn test_string_literal() {
        let input = "\"Hello world!\"";

        test_string_object(test_eval(input), "Hello world!".into());
    }

    #[test]
    fn test_string_concatenation() {
        let input = "\"Hello\" + \" \" + \"world!\"";

        test_string_object(test_eval(input), "Hello world!".into());
    }

    #[test]
    fn test_builtin_functions() {
        let number_tests = [
            (r#"len("")"#, 0),
            (r#"len("four")"#, 4),
            (r#"len("hello world")"#, 11),
            (r#"len([1, 2, 3])"#, 3),
            (r#"first([1, 2, 3])"#, 1),
            (r#"last([1, 2, 3])"#, 3),
        ];
        let error_tests = [
            (r#"len(1)"#, "argument to `len` not supported, got INTEGER"),
            (
                r#"len("one", "two")"#,
                "wrong number of arguments. got=2, want=1",
            ),
        ];

        for (input, output) in number_tests.iter() {
            test_integer_object(test_eval(input), *output);
        }

        for (input, output) in error_tests.iter() {
            test_error_object(test_eval(input), output.to_string());
        }
    }

    #[test]
    fn test_rest_function() {
        let tests = [(r#"rest([1, 2, 3])"#, "[2, 3]"), (r#"rest([2, 3])"#, "[3]")];

        for (input, output) in tests.iter() {
            println!("{} => {}", input, output);
            assert_eq!(test_eval(input).inspect(), *output);
        }
    }

    #[test]
    fn test_array_literals() {
        let input = "[1, 2 * 2, 3 + 3]";

        let evaluated = test_eval(input);

        if let Object::Array(array) = evaluated {
            test_integer_object(array.elements[0].to_owned(), 1);
            test_integer_object(array.elements[1].to_owned(), 4);
            test_integer_object(array.elements[2].to_owned(), 6)
        } else {
            panic!("not a array")
        }
    }

    #[test]
    fn test_array_index_expressions() {
        let tests = [
            ("[1, 2, 3][0]", 1),
            ("[1, 2, 3][1]", 2),
            ("[1, 2, 3][2]", 3),
            ("let i = 0; [1][i];", 1),
            ("[1, 2, 3][1 + 1];", 3),
            ("let myArray = [1, 2, 3]; myArray[2];", 3),
            (
                "let myArray = [1, 2, 3]; myArray[0] + myArray[1] + myArray[2];",
                6,
            ),
            ("let myArray = [1, 2, 3]; let i = myArray[0]; myArray[i]", 2),
        ];

        let null_tests = ["[1, 2, 3][3]", "[1, 2, 3][-1]"];

        for (input, output) in tests.iter() {
            test_integer_object(test_eval(input), *output);
        }

        for input in null_tests {
            test_null_object(test_eval(input));
        }
    }

    #[test]
    fn test_hash_literals() {
        let input = r#"
            let two = "two";
            {
              "one": 10 - 9,
              two: 1 + 1,
              "thr" + "ee": 6 / 2,
              4: 4,
              true: 5,
              false: 6
            }
"#;
        let evaluated = test_eval(input);

        if let Object::Hash(hash) = evaluated {
            let expected = HashMap::from([
                (
                    Str::new("one".into()).hash_key().stringify(),
                    Object::Integer(Integer::new(1)),
                ),
                (
                    Str::new("two".into()).hash_key().stringify(),
                    Object::Integer(Integer::new(2)),
                ),
                (
                    Str::new("three".into()).hash_key().stringify(),
                    Object::Integer(Integer::new(3)),
                ),
                (
                    Integer::new(4).hash_key().stringify(),
                    Object::Integer(Integer::new(4)),
                ),
                (
                    Boolean::new(true).hash_key().stringify(),
                    Object::Integer(Integer::new(5)),
                ),
                (
                    Boolean::new(false).hash_key().stringify(),
                    Object::Integer(Integer::new(6)),
                ),
            ]);
            assert_eq!(hash.value, expected);
        } else {
            panic!("not a hash")
        }
    }
}
