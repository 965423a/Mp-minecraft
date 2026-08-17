//! 内核内置数据库引擎:MariaDB/MySQL 兼容 SQL 子集。
//!
//! 支持:CREATE TABLE / DROP TABLE / SHOW TABLES / INSERT INTO /
//! SELECT(*,列列表,COUNT(*),WHERE,AND,LIMIT)/ UPDATE / DELETE。
//! 表存在堆上(kalloc),行数受限 MAX_ROWS 防止内存耗尽。
//! 由 `sql` 命令执行(systemctl 的 mysqld.service 管理服务状态)。

use alloc::format;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::fmt::Display;

/// 数据库实例:MySQL 与 MariaDB 各自独立(独立表空间/状态)。
pub struct Db {
    tables: Vec<Table>,
    running: bool,
    queries: u64,
}

impl Db {
    const fn new() -> Self {
        Db {
            tables: Vec::new(),
            running: false,
            queries: 0,
        }
    }
}

static mut MYSQL: Db = Db::new();
static mut MARIADB: Db = Db::new();

/// MySQL 实例(mysqld.service)。
pub fn mysql() -> &'static mut Db {
    unsafe { &mut *core::ptr::addr_of_mut!(MYSQL) }
}

/// MariaDB 实例(mariadb.service)。
pub fn mariadb() -> &'static mut Db {
    unsafe { &mut *core::ptr::addr_of_mut!(MARIADB) }
}

/// 服务状态。
pub fn server_running(db: &Db) -> bool {
    db.running
}

pub fn server_start(db: &mut Db) {
    db.running = true;
}

pub fn server_stop(db: &mut Db) {
    db.running = false;
}

pub fn query_count(db: &Db) -> u64 {
    db.queries
}

const MAX_TABLES: usize = 16;
const MAX_ROWS: usize = 4096;
const MAX_COLS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColType {
    Int,
    Double,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Double(f64),
    Text(Vec<u8>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "INT",
            Value::Double(_) => "DOUBLE",
            Value::Text(_) => "TEXT",
        }
    }
}

pub struct Column {
    pub name: String,
    pub ty: ColType,
}

pub struct Table {
    pub name: String,
    pub cols: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}

/// 在某实例上执行一条 SQL 语句,返回全部输出行(表格/结果/错误)。
pub fn execute(db: &mut Db, sql: &str) -> String {
    db.queries += 1;
    let mut out = String::new();
    match Parser::new(sql, &mut out, db).parse() {
        Ok(()) => {}
        Err(e) => {
            let _ = core::fmt::write(&mut out, format_args!("  error: {e}\n"));
        }
    }
    out
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    out: &'a mut dyn core::fmt::Write,
    db: &'a mut Db,
}

#[derive(Debug)]
struct ParseErr(String);

impl Display for ParseErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ParseErr {
    fn from(e: &str) -> Self {
        ParseErr(e.to_string())
    }
}

type PResult<T> = Result<T, ParseErr>;

fn err<T>(msg: &str) -> PResult<T> {
    Err(ParseErr(msg.into()))
}

impl<'a> Parser<'a> {
    fn new(s: &'a str, out: &'a mut dyn core::fmt::Write, db: &'a mut Db) -> Self {
        Parser {
            s: s.as_bytes(),
            pos: 0,
            out,
            db,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.s.len()
            && matches!(self.s[self.pos], b' ' | b'\t' | b'\r' | b'\n')
        {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> u8 {
        self.skip_ws();
        if self.pos < self.s.len() {
            self.s[self.pos]
        } else {
            0
        }
    }

    fn ident(&mut self) -> PResult<String> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len() {
            let c = self.s[self.pos];
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return err("expected identifier");
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.pos]).into_owned())
    }

    fn expect_keyword(&mut self, kw: &str) -> PResult<()> {
        let id = self.ident()?;
        if id.eq_ignore_ascii_case(kw) {
            Ok(())
        } else {
            err(&format!("expected '{kw}', got '{id}'"))
        }
    }

    fn number(&mut self) -> PResult<Value> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len() {
            let c = self.s[self.pos];
            if c.is_ascii_digit() || c == b'.' || c == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return err("expected number");
        }
        let t = String::from_utf8_lossy(&self.s[start..self.pos]);
        if t.contains('.') {
            Ok(Value::Double(t.parse().unwrap_or(0.0)))
        } else {
            Ok(Value::Int(t.parse().unwrap_or(0)))
        }
    }

    fn string_lit(&mut self) -> PResult<Value> {
        self.skip_ws();
        if self.peek() != b'\'' && self.peek() != b'"' {
            return err("expected string literal");
        }
        let q = self.s[self.pos];
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos] != q {
            self.pos += 1;
        }
        if self.pos >= self.s.len() {
            return err("unterminated string");
        }
        let v = self.s[start..self.pos].to_vec();
        self.pos += 1;
        Ok(Value::Text(v))
    }

    fn value(&mut self) -> PResult<Value> {
        self.skip_ws();
        match self.peek() {
            b'\'' | b'"' => self.string_lit(),
            b'0'..=b'9' | b'-' => self.number(),
            _ => err("expected value"),
        }
    }

    fn end(&mut self) -> PResult<()> {
        self.skip_ws();
        if self.pos < self.s.len() {
            err("trailing input")
        } else {
            Ok(())
        }
    }

    fn parse(mut self) -> PResult<()> {
        self.skip_ws();
        if self.pos >= self.s.len() {
            return Ok(());
        }
        let kw = self.ident()?;
        if kw.eq_ignore_ascii_case("create") {
            self.parse_create()
        } else if kw.eq_ignore_ascii_case("drop") {
            self.parse_drop()
        } else if kw.eq_ignore_ascii_case("show") {
            self.parse_show()
        } else if kw.eq_ignore_ascii_case("insert") {
            self.parse_insert()
        } else if kw.eq_ignore_ascii_case("select") {
            self.parse_select()
        } else if kw.eq_ignore_ascii_case("update") {
            self.parse_update()
        } else if kw.eq_ignore_ascii_case("delete") {
            self.parse_delete()
        } else {
            err(&format!("unknown statement '{kw}'"))
        }
    }

    fn parse_create(&mut self) -> PResult<()> {
        self.expect_keyword("table")?;
        let name = self.ident()?.to_ascii_lowercase();
        if self.db.tables.iter().any(|t| t.name == name) {
            return err(&format!("table '{name}' already exists"));
        }
        if self.db.tables.len() >= MAX_TABLES {
            return err("too many tables");
        }
        self.skip_ws();
        if self.peek() != b'(' {
            return err("expected '('");
        }
        self.pos += 1;
        let mut cols: Vec<Column> = Vec::new();
        loop {
            let cname = self.ident()?;
            let ctype = self.ident()?;
            let ty = if ctype.eq_ignore_ascii_case("int") {
                ColType::Int
            } else if ctype.eq_ignore_ascii_case("double") {
                ColType::Double
            } else if ctype.eq_ignore_ascii_case("text")
                || ctype.eq_ignore_ascii_case("varchar")
            {
                ColType::Text
            } else {
                return err(&format!("unknown type '{ctype}'"));
            };
            cols.push(Column {
                name: cname.to_ascii_lowercase(),
                ty,
            });
            self.skip_ws();
            if self.peek() == b',' {
                self.pos += 1;
                continue;
            }
            if self.peek() == b')' {
                self.pos += 1;
                break;
            }
            return err("expected ',' or ')'");
        }
        if cols.is_empty() || cols.len() > MAX_COLS {
            return err("bad column count");
        }
        self.db.tables.push(Table {
            name,
            cols,
            rows: Vec::new(),
        });
        let _ = core::fmt::write(self.out, format_args!("  table created\n"));
        Ok(())
    }

    fn parse_drop(&mut self) -> PResult<()> {
        self.expect_keyword("table")?;
        let name = self.ident()?.to_ascii_lowercase();
        let before = self.db.tables.len();
        self.db.tables.retain(|t| t.name != name);
        if self.db.tables.len() == before {
            return err(&format!("table '{name}' not found"));
        }
        let _ = core::fmt::write(self.out, format_args!("  table dropped\n"));
        Ok(())
    }

    fn parse_show(&mut self) -> PResult<()> {
        self.expect_keyword("tables")?;
        if self.db.tables.is_empty() {
            let _ = core::fmt::write(self.out, format_args!("  (no tables)\n"));
            return Ok(());
        }
        let _ = core::fmt::write(
            self.out,
            format_args!("  +-------------------------+\n"),
        );
        let _ = core::fmt::write(
            self.out,
            format_args!("  | Tables_in_kernel        |\n"),
        );
        let _ = core::fmt::write(
            self.out,
            format_args!("  +-------------------------+\n"),
        );
        for t in self.db.tables.iter() {
            let _ = core::fmt::write(
                self.out,
                format_args!("  | {:<24}| \n", t.name),
            );
        }
        let _ = core::fmt::write(
            self.out,
            format_args!("  +-------------------------+\n"),
        );
        Ok(())
    }

    fn parse_insert(&mut self) -> PResult<()> {
        self.expect_keyword("into")?;
        let name = self.ident()?.to_ascii_lowercase();
        let idx = match self.db.tables.iter().position(|t| t.name == name) {
            Some(i) => i,
            None => return err(&format!("table '{name}' not found")),
        };
        self.expect_keyword("values")?;
        let ncols = self.db.tables[idx].cols.len();
        loop {
            self.skip_ws();
            if self.peek() != b'(' {
                return err("expected '('");
            }
            self.pos += 1;
            let mut row: Vec<Value> = Vec::new();
            loop {
                let v = self.value()?;
                row.push(v);
                self.skip_ws();
                if self.peek() == b',' {
                    self.pos += 1;
                    continue;
                }
                if self.peek() == b')' {
                    self.pos += 1;
                    break;
                }
                return err("expected ',' or ')'");
            }
            if row.len() != ncols {
                return err(&format!("expected {ncols} values, got {}", row.len()));
            }
            if self.db.tables[idx].rows.len() >= MAX_ROWS {
                return err("table full");
            }
            self.db.tables[idx].rows.push(row);
            self.skip_ws();
            if self.peek() == b',' {
                self.pos += 1;
                continue;
            }
            break;
        }
        let _ = core::fmt::write(self.out, format_args!("  rows inserted\n"));
        Ok(())
    }

    fn parse_select(&mut self) -> PResult<()> {
        // SELECT <cols> FROM <table> [WHERE <cond>] [LIMIT n]
        let mut sel: Vec<String> = Vec::new();
        let mut count_all = false;
        self.skip_ws();
        if self.peek() == b'*' {
            self.pos += 1;
            sel.push("*".into());
        } else {
            loop {
                let c = self.ident()?;
                if c.eq_ignore_ascii_case("count") {
                    self.skip_ws();
                    if self.peek() == b'(' {
                        self.pos += 1;
                        if self.peek() == b'*' {
                            self.pos += 1;
                            count_all = true;
                        } else {
                            let _ = self.ident()?;
                        }
                        self.skip_ws();
                        if self.peek() == b')' {
                            self.pos += 1;
                        }
                    }
                    sel.push("count(*)".into());
                } else {
                    sel.push(c);
                }
                self.skip_ws();
                if self.peek() == b',' {
                    self.pos += 1;
                    continue;
                }
                break;
            }
        }
        self.expect_keyword("from")?;
        let name = self.ident()?.to_ascii_lowercase();
        let idx = match self.db.tables.iter().position(|t| t.name == name) {
            Some(i) => i,
            None => return err(&format!("table '{name}' not found")),
        };
        // WHERE
        let mut cond_col: Option<(usize, u8, Value)> = None;
        self.skip_ws();
        if self.pos < self.s.len() {
            let save = self.pos;
            if self.ident()?.eq_ignore_ascii_case("where") {
                let cname = self.ident()?;
                let ci = self.col_index_in(idx, &cname)?;
                self.skip_ws();
                let op = self.peek();
                if !matches!(op, b'=' | b'>' | b'<') {
                    return err("expected comparison operator");
                }
                self.pos += 1;
                if op == b'=' && self.peek() == b'=' {
                    self.pos += 1;
                }
                let v = self.value()?;
                cond_col = Some((ci, op, v));
            } else {
                self.pos = save;
            }
        }
        // LIMIT
        let mut limit: Option<usize> = None;
        self.skip_ws();
        if self.pos < self.s.len() {
            let save = self.pos;
            if self.ident()?.eq_ignore_ascii_case("limit") {
                let n = self.number()?;
                if let Value::Int(n) = n {
                    limit = Some(n.max(0) as usize);
                }
            } else {
                self.pos = save;
            }
        }
        self.end()?;

        let t = &self.db.tables[idx];
        if count_all {
            let _ = core::fmt::write(self.out, format_args!("  +-------+\n"));
            let _ = core::fmt::write(self.out, format_args!("  | count |\n"));
            let _ = core::fmt::write(self.out, format_args!("  +-------+\n"));
            let _ = core::fmt::write(
                self.out,
                format_args!("  | {:5} |\n", t.rows.len()),
            );
            let _ = core::fmt::write(self.out, format_args!("  +-------+\n"));
            return Ok(());
        }
        // 表头
        let col_idx: Vec<usize> = if sel.len() == 1 && sel[0] == "*" {
            (0..t.cols.len()).collect()
        } else {
            let mut v = Vec::new();
            for c in &sel {
                v.push(self.col_index_in(idx, c)?);
            }
            v
        };
        let w = col_idx
            .iter()
            .map(|&i| t.cols[i].name.len().max(4))
            .max()
            .unwrap_or(4);
        let _ = core::fmt::write(self.out, format_args!("  +"));
        for _ in 0..col_idx.len() {
            for _ in 0..w + 2 {
                let _ = self.out.write_char('-');
            }
            let _ = self.out.write_char('+');
        }
        let _ = self.out.write_char('\n');
        let _ = self.out.write_str("  |");
        for &i in &col_idx {
            let _ = core::fmt::write(
                self.out,
                format_args!(" {:^w$} |", t.cols[i].name, w = w),
            );
        }
        let _ = self.out.write_char('\n');
        let _ = core::fmt::write(self.out, format_args!("  +"));
        for _ in 0..col_idx.len() {
            for _ in 0..w + 2 {
                let _ = self.out.write_char('-');
            }
            let _ = self.out.write_char('+');
        }
        let _ = self.out.write_char('\n');
        let mut shown = 0usize;
        for row in t.rows.iter() {
            if let Some((ci, op, want)) = &cond_col {
                if !match_val(&row[*ci], *op, want) {
                    continue;
                }
            }
            if let Some(l) = limit {
                if shown >= l {
                    break;
                }
            }
            let _ = self.out.write_str("  |");
            for &i in &col_idx {
                let _ = core::fmt::write(
                    self.out,
                    format_args!(" {:^w$} |", fmt_val(&row[i]), w = w),
                );
            }
            let _ = self.out.write_char('\n');
            shown += 1;
        }
        let _ = core::fmt::write(self.out, format_args!("  +"));
        for _ in 0..col_idx.len() {
            for _ in 0..w + 2 {
                let _ = self.out.write_char('-');
            }
            let _ = self.out.write_char('+');
        }
        let _ = self.out.write_char('\n');
        let _ = core::fmt::write(
            self.out,
            format_args!("  {} row(s)\n", shown),
        );
        Ok(())
    }

    fn col_index_in(&self, idx: usize, name: &str) -> PResult<usize> {
        let cname = name.to_ascii_lowercase();
        self.db.tables[idx]
            .cols
            .iter()
            .position(|c| c.name == cname)
            .ok_or_else(|| ParseErr(format!("unknown column '{name}'")))
    }

    fn parse_update(&mut self) -> PResult<()> {
        let name = self.ident()?.to_ascii_lowercase();
        let idx = match self.db.tables.iter().position(|t| t.name == name) {
            Some(i) => i,
            None => return err(&format!("table '{name}' not found")),
        };
        self.expect_keyword("set")?;
        let mut sets: Vec<(usize, Value)> = Vec::new();
        loop {
            let cname = self.ident()?;
            let ci = self.col_index_in(idx, &cname)?;
            self.skip_ws();
            if self.peek() != b'=' {
                return err("expected '='");
            }
            self.pos += 1;
            let v = self.value()?;
            sets.push((ci, v));
            self.skip_ws();
            if self.peek() == b',' {
                self.pos += 1;
                continue;
            }
            break;
        }
        let mut cond: Option<(usize, u8, Value)> = None;
        self.skip_ws();
        if self.pos < self.s.len() {
            let save = self.pos;
            if self.ident()?.eq_ignore_ascii_case("where") {
                let cname = self.ident()?;
                let ci = self.col_index_in(idx, &cname)?;
                self.skip_ws();
                let op = self.peek();
                if !matches!(op, b'=' | b'>' | b'<') {
                    return err("expected comparison operator");
                }
                self.pos += 1;
                if op == b'=' && self.peek() == b'=' {
                    self.pos += 1;
                }
                let v = self.value()?;
                cond = Some((ci, op, v));
            } else {
                self.pos = save;
            }
        }
        self.end()?;
        let mut n = 0usize;
        for row in self.db.tables[idx].rows.iter_mut() {
            if let Some((ci, op, want)) = &cond {
                if !match_val(&row[*ci], *op, want) {
                    continue;
                }
            }
            for (ci, v) in &sets {
                row[*ci] = v.clone();
            }
            n += 1;
        }
        let _ = core::fmt::write(self.out, format_args!("  {n} row(s) updated\n"));
        Ok(())
    }

    fn parse_delete(&mut self) -> PResult<()> {
        self.expect_keyword("from")?;
        let name = self.ident()?.to_ascii_lowercase();
        let idx = match self.db.tables.iter().position(|t| t.name == name) {
            Some(i) => i,
            None => return err(&format!("table '{name}' not found")),
        };
        let mut cond: Option<(usize, u8, Value)> = None;
        self.skip_ws();
        if self.pos < self.s.len() {
            let save = self.pos;
            if self.ident()?.eq_ignore_ascii_case("where") {
                let cname = self.ident()?;
                let ci = self.col_index_in(idx, &cname)?;
                self.skip_ws();
                let op = self.peek();
                if !matches!(op, b'=' | b'>' | b'<') {
                    return err("expected comparison operator");
                }
                self.pos += 1;
                if op == b'=' && self.peek() == b'=' {
                    self.pos += 1;
                }
                let v = self.value()?;
                cond = Some((ci, op, v));
            } else {
                self.pos = save;
            }
        }
        self.end()?;
        let before = self.db.tables[idx].rows.len();
        self.db.tables[idx]
            .rows
            .retain(|row| match &cond {
                Some((ci, op, want)) => match_val(&row[*ci], *op, want),
                None => false,
            });
        let _ = core::fmt::write(
            self.out,
            format_args!("  {} row(s) deleted\n", before - self.db.tables[idx].rows.len()),
        );
        Ok(())
    }
}

/// 引擎自检:在指定实例上跑完整 CRUD 冒烟测试。
pub fn selftest(db: &mut Db) -> String {
    let mut all = String::new();
    let cases = [
        "create table users (id int, name text, score double)",
        "insert into users values (1, 'alice', 95.5)",
        "insert into users values (2, 'bob', 87.0)",
        "insert into users values (3, 'carol', 99.9)",
        "select * from users",
        "select name, score from users where score > 90",
        "select count(*) from users",
        "update users set score = 100.0 where name = 'bob'",
        "select * from users",
        "delete from users where id = 3",
        "select count(*) from users",
        "show tables",
        "drop table users",
        "show tables",
    ];
    for c in cases {
        let _ = core::fmt::write(&mut all, format_args!("sql> {c}\n"));
        all.push_str(&execute(db, c));
    }
    all
}

/// 隔离断言:一个实例的表空间不得影响另一个。
/// 用法:先往 mysql 建表,再确认 mariadb 为空。
pub fn isolation_ok() -> bool {
    let m = unsafe { &mut *core::ptr::addr_of_mut!(MYSQL) };
    let r = unsafe { &mut *core::ptr::addr_of_mut!(MARIADB) };
    m.tables.iter().any(|t| t.name == "isoprobe")
        && r.tables.iter().all(|t| t.name != "isoprobe")
}

fn match_val(actual: &Value, op: u8, want: &Value) -> bool {
    let cmp = match (actual, want) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b) as i8,
        (Value::Int(a), Value::Double(b)) => cmp_f64(*a as f64, *b),
        (Value::Double(a), Value::Int(b)) => cmp_f64(*a, *b as f64),
        (Value::Double(a), Value::Double(b)) => cmp_f64(*a, *b),
        (Value::Text(a), Value::Text(b)) => a.cmp(b) as i8,
        _ => return false,
    };
    match op {
        b'=' => cmp == 0,
        b'>' => cmp > 0,
        b'<' => cmp < 0,
        _ => false,
    }
}

fn cmp_f64(a: f64, b: f64) -> i8 {
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        0
    }
}

fn fmt_val(v: &Value) -> String {
    match v {
        Value::Int(i) => format!("{i}"),
        Value::Double(d) => format!("{d}"),
        Value::Text(t) => String::from_utf8_lossy(t).into_owned(),
    }
}
