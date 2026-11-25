//! Small, bounded expression evaluator. Never evaluates external code.
use crate::store::Result;

const MAX_EXPR: usize = 4096;
const MAX_NODES: usize = 512;
const MAX_DEPTH: usize = 32;
const MAX_STRING: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value { Num(f64), Bool(bool), Str(String), Date(String) }
#[derive(Debug)]
enum Expr { Literal(Value), Unary(char,Box<Expr>), Binary(&'static str,Box<Expr>,Box<Expr>), Call(String,Vec<Expr>) }
struct Parser { chars: Vec<char>, pos: usize, nodes: usize }
impl Parser {
    fn skip(&mut self) { while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) { self.pos += 1; } }
    fn consume(&mut self, token: &str) -> bool {
        self.skip(); let chars: Vec<char> = token.chars().collect();
        if self.chars[self.pos..].starts_with(&chars) { self.pos += chars.len(); true } else { false }
    }
    fn node(&mut self, e: Expr) -> Result<Expr> { self.nodes += 1; if self.nodes > MAX_NODES { return Err("公式過於複雜。".into()); } Ok(e) }
    fn expression(&mut self, depth: usize) -> Result<Expr> { self.comparison(depth) }
    fn comparison(&mut self, depth: usize) -> Result<Expr> {
        let mut e = self.add(depth)?;
        loop {
            let op = ["==","!=","<=",">=","<",">"].into_iter().find(|op| self.consume(op));
            let Some(op) = op else { break }; let rhs = self.add(depth)?;
            e = self.node(Expr::Binary(op,Box::new(e),Box::new(rhs)))?;
        } Ok(e)
    }
    fn add(&mut self, depth: usize) -> Result<Expr> {
        let mut e = self.multiply(depth)?;
        loop { let op = if self.consume("+") { "+" } else if self.consume("-") { "-" } else { break };
            let rhs=self.multiply(depth)?; e=self.node(Expr::Binary(op,Box::new(e),Box::new(rhs)))?; } Ok(e)
    }
    fn multiply(&mut self, depth: usize) -> Result<Expr> {
        let mut e = self.atom(depth)?;
        loop { let op = if self.consume("*") { "*" } else if self.consume("/") { "/" } else { break };
            let rhs=self.atom(depth)?; e=self.node(Expr::Binary(op,Box::new(e),Box::new(rhs)))?; } Ok(e)
    }
    fn atom(&mut self, depth: usize) -> Result<Expr> {
        if depth >= MAX_DEPTH { return Err("公式巢狀過深。".into()); }
        self.skip();
        if self.consume("-") { let e=self.atom(depth+1)?; return self.node(Expr::Unary('-',Box::new(e))); }
        if self.consume("(") { let e=self.expression(depth+1)?; if !self.consume(")") { return Err("缺少右括號。".into()); } return Ok(e); }
        if self.consume("\"") {
            let mut value=String::new(); let mut closed=false;
            while let Some(&c)=self.chars.get(self.pos) { self.pos+=1; if c=='"' { closed=true; break; }
                if c=='\\' { let n=*self.chars.get(self.pos).ok_or("字串跳脫不完整。")?; self.pos+=1;
                    value.push(match n { '"'=>'"', '\\'=>'\\', 'n'=>'\n', 'r'=>'\r', 't'=>'\t', _=>return Err("不支援的字串跳脫。".into()) });
                } else { value.push(c); }
                if value.len()>MAX_STRING { return Err("公式字串過長。".into()); }
            }
            if !closed { return Err("字串未結束。".into()); } return self.node(Expr::Literal(Value::Str(value)));
        }
        self.skip();
        let start=self.pos;
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit() || *c=='.') { self.pos+=1; }
        if self.pos>start { let n: String=self.chars[start..self.pos].iter().collect(); let n: f64=n.parse().map_err(|_| "數字格式不正確。")?;
            if !n.is_finite() { return Err("數字超出範圍。".into()); } return self.node(Expr::Literal(Value::Num(n))); }
        while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_alphabetic() || *c=='_') { self.pos+=1; }
        if self.pos>start {
            let name: String=self.chars[start..self.pos].iter().collect();
            if name=="true" || name=="false" { return self.node(Expr::Literal(Value::Bool(name=="true"))); }
            if !self.consume("(") { return Err("未知名稱。".into()); }
            let mut args=Vec::new(); if !self.consume(")") { loop {
                if args.len()>=3 { return Err("函式參數過多。".into()); }
                args.push(self.expression(depth+1)?);
                if self.consume(")") { break; } if !self.consume(",") { return Err("函式參數格式錯誤。".into()); }
            }}
            return self.node(Expr::Call(name,args));
        }
        Err("公式格式錯誤。".into())
    }
}
fn number(v: Value) -> Result<f64> { if let Value::Num(n)=v {Ok(n)} else {Err("此操作需要數字；屬性請使用 number() 轉換。".into())} }
fn run(e: &Expr, lookup: &impl Fn(&str,&[String])->Result<Value>, depth: usize) -> Result<Value> {
    if depth>MAX_NODES { return Err("公式運算過深。".into()); }
    match e {
        Expr::Literal(v)=>Ok(v.clone()),
        Expr::Unary('-',v)=>Ok(Value::Num(-number(run(v,lookup,depth+1)?)?)),
        Expr::Unary(_,_)=>Err("未知運算。".into()),
        Expr::Call(name,args) if name=="if" && args.len()==3 => {
            let Value::Bool(test)=run(&args[0],lookup,depth+1)? else { return Err("if 條件必須為布林值。".into()); };
            run(&args[if test {1} else {2}],lookup,depth+1)
        },
        Expr::Call(name,args) if matches!(name.as_str(),"prop"|"typed"|"relation_count"|"relation_sum"|"date") => {
            let mut names = Vec::with_capacity(args.len());
            for arg in args {
                let Value::Str(text) = run(arg,lookup,depth+1)? else {
                    return Err(format!("{name} 的參數需要文字。").into());
                };
                names.push(text);
            }
            if name == "date" {
                let day = crate::store::typed_properties::PropertyType::Date.normalized(&names[0])
                    .ok_or("date 需要有效的 YYYY-MM-DD 日期。")?;
                return Ok(Value::Date(day));
            }
            lookup(name,&names)
        },
        Expr::Call(name,args) if name=="number" && args.len()==1 => {
            let Value::Str(text)=run(&args[0],lookup,depth+1)? else { return Err("number 需要文字。".into()); };
            let n: f64=text.trim().parse().map_err(|_| "無法轉換成數字。")?;
            if !n.is_finite() { return Err("數字超出範圍。".into()); } Ok(Value::Num(n))
        },
        Expr::Call(_,_)=>Err("未知函式或參數數量錯誤。".into()),
        Expr::Binary(op,left,right)=>{
            let a=run(left,lookup,depth+1)?; let b=run(right,lookup,depth+1)?;
            if matches!(*op,"=="|"!=") { let same=match (&a,&b) { (Value::Num(x),Value::Num(y))=>x==y,(Value::Str(x),Value::Str(y))|(Value::Date(x),Value::Date(y))=>x==y,(Value::Bool(x),Value::Bool(y))=>x==y,_=>return Err("比較的類型必須相同。".into()) }; return Ok(Value::Bool(if *op=="==" {same} else {!same})); }
            if let (Value::Date(x),Value::Date(y)) = (&a,&b) {
                return match *op {
                    "<" => Ok(Value::Bool(x < y)), ">" => Ok(Value::Bool(x > y)),
                    "<=" => Ok(Value::Bool(x <= y)), ">=" => Ok(Value::Bool(x >= y)),
                    _ => Err("日期只支援比較運算。".into()),
                };
            }
            let x=number(a)?; let y=number(b)?;
            if matches!(*op,"<"|">"|"<="|">=") { return Ok(Value::Bool(match *op {"<"=>x<y,">"=>x>y,"<="=>x<=y,_=>x>=y})); }
            if *op=="/" && y==0.0 { return Err("不能除以零。".into()); }
            let n=match *op {"+"=>x+y,"-"=>x-y,"*"=>x*y,"/"=>x/y,_=>return Err("未知運算。".into())};
            if !n.is_finite() { return Err("運算結果超出範圍。".into()); } Ok(Value::Num(n))
        }
    }
}
fn parse(expr: &str) -> Result<Expr> {
    if expr.len()>MAX_EXPR || expr.contains('\0') { return Err("公式過長或包含 NUL。".into()); }
    let mut p=Parser {chars:expr.chars().collect(),pos:0,nodes:0}; let ast=p.expression(0)?; p.skip();
    if p.pos!=p.chars.len() { return Err("公式包含多餘內容。".into()); }
    check_calls(&ast)?;
    Ok(ast)
}
fn check_calls(e: &Expr) -> Result<()> {
    match e {
        Expr::Literal(_) => Ok(()),
        Expr::Unary(_,x) => check_calls(x),
        Expr::Binary(_,a,b) => { check_calls(a)?; check_calls(b) },
        Expr::Call(name,args) => {
            let valid = matches!((name.as_str(),args.len()), ("if",3)|("prop"|"number"|"date"|"typed"|"relation_count",1)|("relation_sum",2));
            if !valid { return Err("未知函式或參數數量錯誤。".into()); }
            for arg in args { check_calls(arg)?; } Ok(())
        }
    }
}
pub(crate) struct CompiledFormula(Expr);

impl CompiledFormula {
    pub fn new(expr: &str) -> Result<Self> { Ok(Self(parse(expr)?)) }

    pub fn evaluate(&self, lookup: impl Fn(&str, &[String]) -> Result<Value>) -> Result<Value> {
        run(&self.0, &lookup, 0)
    }
}

impl Value {
    pub(crate) fn into_display(self) -> String {
        match self { Self::Num(n) => n.to_string(), Self::Bool(v) => v.to_string(), Self::Str(v) | Self::Date(v) => v }
    }
}

/// Validate grammar and supported function arity without looking up properties or evaluating.
pub fn validate(expr: &str) -> Result<()> { CompiledFormula::new(expr).map(|_| ()) }

#[cfg(test)]
fn evaluate_with(expr: &str, lookup: impl Fn(&str, &[String]) -> Result<Value>) -> Result<String> {
    Ok(CompiledFormula::new(expr)?.evaluate(lookup)?.into_display())
}

#[cfg(test)]
fn evaluate(expr: &str, lookup: impl Fn(&str)->Option<String>) -> Result<String> {
    evaluate_with(expr,|name,args| {
        if name != "prop" { return Err("函式需要筆記內容。".into()); }
        Ok(Value::Str(lookup(&args[0]).ok_or("找不到指定屬性。")?))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn arithmetic_properties_and_lazy_if() -> Result<()> {
        let get=|k:&str| if k=="數量" {Some("12".to_owned())} else {None};
        assert_eq!(evaluate("number(prop(\"數量\"))*2+1",get)?,"25");
        assert_eq!(evaluate("if(2 > 1, \"好\", 1 / 0)",get)?,"好");
        assert_eq!(evaluate("if(false, prop(\"missing\"), true)",get)?,"true"); Ok(())
    }
    #[test] fn errors_and_budgets() {
        for e in ["1/0","prop(\"missing\")","number(\"bad\")","1 + true","1 +", "unknown(1)","1 2"] { assert!(evaluate(e, |_| None).is_err(),"{e}"); }
        assert!(validate("number(prop(\"未建立\"))").is_ok());
        assert!(validate("unknown(1)").is_err());
        assert!(validate("if(true,1)").is_err());
        assert!(evaluate(&format!("{}1{}","(".repeat(40),")".repeat(40)), |_| None).is_err());
        assert!(evaluate(&"1+".repeat(300), |_| None).is_err());
    }
}
