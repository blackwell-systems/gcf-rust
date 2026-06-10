use std::io::Write;
use std::sync::Mutex;

struct SectionCount {
    #[allow(dead_code)]
    name: String,
    count: usize,
}

struct ActiveArray {
    name: String,
    #[allow(dead_code)]
    fields: Vec<String>,
    count: usize,
}

struct GenericStreamEncoderInner<W: Write> {
    w: W,
    sections: Vec<SectionCount>,
    current: Option<ActiveArray>,
}

/// GenericStreamEncoder writes GCF tabular output incrementally as rows arrive.
/// Zero buffering: each row is written immediately. A trailer summary is
/// emitted on close() with the final counts.
///
/// Thread-safe via internal Mutex.
///
/// # Example
///
/// ```
/// use gcf::GenericStreamEncoder;
/// use gcf::stream_generic::GcfValue;
///
/// let buf = Vec::new();
/// let enc = GenericStreamEncoder::new(buf);
/// enc.begin_array("employees", &["id", "name", "department", "salary"]);
/// enc.write_row(&[1.into(), "Alice".into(), "Engineering".into(), 95000.into()]);
/// enc.end_array();
/// enc.close().unwrap();
/// ```
pub struct GenericStreamEncoder<W: Write> {
    inner: Mutex<GenericStreamEncoderInner<W>>,
}

/// Value types that can be written as GCF row values.
#[derive(Clone, Debug)]
pub enum GcfValue {
    /// Null value, rendered as `-`
    Null,
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i64),
    /// Floating-point value
    Float(f64),
    /// String value
    Str(String),
}

impl GcfValue {
    fn format(&self) -> String {
        match self {
            GcfValue::Null => "-".to_string(),
            GcfValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            GcfValue::Int(n) => n.to_string(),
            GcfValue::Float(f) => format!("{}", f),
            GcfValue::Str(s) => {
                if s.is_empty() {
                    return "\"\"".to_string();
                }
                if s.contains('|') || s.contains('\n') {
                    return format!("\"{}\"", s.replace('"', "\\\""));
                }
                s.clone()
            }
        }
    }
}

impl From<i32> for GcfValue {
    fn from(v: i32) -> Self {
        GcfValue::Int(v as i64)
    }
}

impl From<i64> for GcfValue {
    fn from(v: i64) -> Self {
        GcfValue::Int(v)
    }
}

impl From<f64> for GcfValue {
    fn from(v: f64) -> Self {
        GcfValue::Float(v)
    }
}

impl From<bool> for GcfValue {
    fn from(v: bool) -> Self {
        GcfValue::Bool(v)
    }
}

impl From<&str> for GcfValue {
    fn from(v: &str) -> Self {
        GcfValue::Str(v.to_string())
    }
}

impl From<String> for GcfValue {
    fn from(v: String) -> Self {
        GcfValue::Str(v)
    }
}

impl<W: Write> GenericStreamEncoder<W> {
    /// Create a new streaming encoder for tabular/generic data.
    pub fn new(w: W) -> Self {
        GenericStreamEncoder {
            inner: Mutex::new(GenericStreamEncoderInner {
                w,
                sections: Vec::new(),
                current: None,
            }),
        }
    }

    /// Start a tabular array section with deferred count [?].
    pub fn begin_array(&self, name: &str, fields: &[&str]) {
        let mut inner = self.inner.lock().unwrap();
        if inner.current.is_some() {
            Self::end_array_locked(&mut inner);
        }
        let fields_str = fields.join(",");
        writeln!(inner.w, "## {} [?]{{{}}}", name, fields_str).unwrap();
        inner.current = Some(ActiveArray {
            name: name.to_string(),
            fields: fields.iter().map(|s| s.to_string()).collect(),
            count: 0,
        });
    }

    /// Emit a single pipe-separated row immediately.
    pub fn write_row(&self, values: &[GcfValue]) {
        let mut inner = self.inner.lock().unwrap();
        if inner.current.is_none() {
            return;
        }
        let parts: Vec<String> = values.iter().map(|v| v.format()).collect();
        writeln!(inner.w, "{}", parts.join("|")).unwrap();
        if let Some(ref mut current) = inner.current {
            current.count += 1;
        }
    }

    /// Close the current array section and record its count.
    pub fn end_array(&self) {
        let mut inner = self.inner.lock().unwrap();
        Self::end_array_locked(&mut inner);
    }

    /// Emit a key=value line immediately.
    pub fn write_kv(&self, key: &str, value: &GcfValue) {
        let mut inner = self.inner.lock().unwrap();
        writeln!(inner.w, "{}={}", key, value.format()).unwrap();
    }

    /// Start a nested object section (## key).
    pub fn write_section(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.current.is_some() {
            Self::end_array_locked(&mut inner);
        }
        writeln!(inner.w, "## {}", name).unwrap();
    }

    /// Emit a primitive array inline: name[N]: val1,val2,val3
    pub fn write_inline_array(&self, name: &str, values: &[GcfValue]) {
        let mut inner = self.inner.lock().unwrap();
        let parts: Vec<String> = values.iter().map(|v| v.format()).collect();
        writeln!(inner.w, "{}[{}]: {}", name, values.len(), parts.join(",")).unwrap();
    }

    /// Emit the ##! summary trailer with final counts.
    pub fn close(&self) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.current.is_some() {
            Self::end_array_locked(&mut inner);
        }
        if inner.sections.is_empty() {
            return Ok(());
        }
        let counts: Vec<String> = inner.sections.iter().map(|s| s.count.to_string()).collect();
        writeln!(inner.w, "##! summary counts={}", counts.join(","))?;
        Ok(())
    }

    fn end_array_locked(inner: &mut GenericStreamEncoderInner<W>) {
        if let Some(current) = inner.current.take() {
            inner.sections.push(SectionCount {
                name: current.name,
                count: current.count,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tabular() {
        let buf = Vec::new();
        let enc = GenericStreamEncoder::new(buf);
        enc.begin_array("employees", &["id", "name", "department", "salary"]);
        enc.write_row(&[1.into(), "Alice".into(), "Engineering".into(), 95000.into()]);
        enc.write_row(&[2.into(), "Bob".into(), "Sales".into(), 72000.into()]);
        enc.write_row(&[3.into(), "Carol".into(), "Marketing".into(), 85000.into()]);
        enc.end_array();
        enc.close().unwrap();

        let inner = enc.inner.lock().unwrap();
        let out = String::from_utf8(inner.w.clone()).unwrap();
        assert!(out.contains("## employees [?]{id,name,department,salary}"));
        assert!(out.contains("1|Alice|Engineering|95000"));
        assert!(out.contains("##! summary counts=3"));
    }

    #[test]
    fn test_kv_and_inline_array() {
        let buf = Vec::new();
        let enc = GenericStreamEncoder::new(buf);
        enc.write_kv("name", &"my-service".into());
        enc.write_kv("version", &"2.1.0".into());
        enc.write_inline_array(
            "tags",
            &["production".into(), "us-east-1".into(), "critical".into()],
        );
        enc.close().unwrap();

        let inner = enc.inner.lock().unwrap();
        let out = String::from_utf8(inner.w.clone()).unwrap();
        assert!(out.contains("name=my-service"));
        assert!(out.contains("tags[3]: production,us-east-1,critical"));
    }

    #[test]
    fn test_multiple_arrays() {
        let buf = Vec::new();
        let enc = GenericStreamEncoder::new(buf);
        enc.begin_array("users", &["id", "name"]);
        enc.write_row(&[1.into(), "Alice".into()]);
        enc.write_row(&[2.into(), "Bob".into()]);
        enc.end_array();
        enc.begin_array("roles", &["name", "level"]);
        enc.write_row(&["admin".into(), 10.into()]);
        enc.end_array();
        enc.close().unwrap();

        let inner = enc.inner.lock().unwrap();
        let out = String::from_utf8(inner.w.clone()).unwrap();
        assert!(out.contains("counts=2,1"));
    }

    #[test]
    fn test_null_and_bool() {
        let buf = Vec::new();
        let enc = GenericStreamEncoder::new(buf);
        enc.begin_array("data", &["a", "b", "c"]);
        enc.write_row(&[GcfValue::Null, true.into(), false.into()]);
        enc.end_array();
        enc.close().unwrap();

        let inner = enc.inner.lock().unwrap();
        let out = String::from_utf8(inner.w.clone()).unwrap();
        assert!(out.contains("-|true|false"));
    }

    #[test]
    fn test_incremental() {
        let buf = Vec::new();
        let enc = GenericStreamEncoder::new(buf);
        enc.begin_array("data", &["id", "val"]);
        {
            let inner = enc.inner.lock().unwrap();
            assert!(!inner.w.is_empty(), "header should be written immediately");
        }
        enc.write_row(&[1.into(), "a".into()]);
        enc.end_array();
        enc.close().unwrap();
    }
}
