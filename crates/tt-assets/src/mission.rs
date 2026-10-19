//! Mission and def objects as flat records ([`SceneObject`]): what the
//! RON data files ([`crate::data`]) declare, in the form the world and
//! scene builders read.

#[derive(Debug, Clone)]
pub struct SceneObject {
    pub class: String,
    pub name: String,
    pub def: bool,
    /// Object this one copies its fields from (`new X(Name : Parent)`).
    pub parent: Option<String>,
    /// Field name/value pairs in file order.
    pub fields: Vec<(String, String)>,
    pub children: Vec<SceneObject>,
}

impl SceneObject {
    pub fn get(&self, field: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(field)).map(|(_, v)| v.as_str())
    }

    pub fn floats(&self, field: &str) -> Vec<f32> {
        self.get(field).map(|v| v.split_whitespace().filter_map(|t| t.parse().ok()).collect()).unwrap_or_default()
    }

    /// Depth-first iteration over this object and all descendants.
    pub fn walk(&self) -> Vec<&SceneObject> {
        let mut out = vec![self];
        for c in &self.children {
            out.extend(c.walk());
        }
        out
    }
}

/// Merges the fields of `block`'s parent def (`def X(B: P)`), looked up
/// by name in `known`, under its own.
pub fn inherit(block: &mut SceneObject, known: &[SceneObject]) {
    let Some(p) = block.parent.clone() else { return };
    let Some(parent) = known.iter().rev().find(|b| b.name.eq_ignore_ascii_case(&p)) else { return };
    let mut fields = parent.fields.clone();
    for (k, v) in &block.fields {
        match fields.iter_mut().find(|(pk, _)| pk.eq_ignore_ascii_case(k)) {
            Some(slot) => slot.1 = v.clone(),
            None => fields.push((k.clone(), v.clone())),
        }
    }
    block.fields = fields;
}
