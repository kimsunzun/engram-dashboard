//! The order-keeping YAML document tree the preset writer rewrites (ADR-0358). `serde_json::Value` cannot be it: without
//! `preserve_order` its maps sort their keys, and it holds no non-string key (TRD S25 step-7 §2 premise 38).
//!
//! What the tree does not hold, by decision: comments, quoting and flow style, anchors and aliases (expanded), merge
//! keys (expanded), explicit tags (the tagged value stays), the sign of a NaN.
// ADR-0358

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

/// One YAML node. A map keeps its entries in file order and may have keys of any kind.
///
/// `Int` holds what the reader hands over as an integer — always within `i64` or `u64`. A decimal integer literal
/// outside those ranges reaches the tree already turned into a `Float` by the reader (premise 39) — rounded, or an
/// infinity beyond the `f64` range; the writer refuses both instead of writing them ([`super::write::rewrite_lists`]).
#[derive(Clone, Debug)]
pub enum Node {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

impl Node {
    /// Structural equality: maps compare entry by entry in order. Floats compare by bits, except that every NaN equals
    /// every NaN (the writer does not keep a NaN's sign or payload) — so `-0.0` and `0.0` differ.
    pub fn same(&self, other: &Node) -> bool {
        self.first_difference(other).is_none()
    }

    /// The path (`/`-separated keys and indices, `/` = the root) of the first node where `self` and `other` differ
    /// under [`Node::same`]; `None` = they are the same.
    pub(crate) fn first_difference(&self, other: &Node) -> Option<String> {
        let mut path = Vec::new();
        differ(self, other, &mut path).then(|| join_path(&path))
    }

    /// The text of a string node.
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(text) => Some(text),
            _ => None,
        }
    }

    /// The value of the first entry of a map node whose key is the string `key`.
    pub(crate) fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .map(|(_, value)| value),
            _ => None,
        }
    }
}

/// Whether `a` and `b` differ; on `true`, `path` holds the path of the first differing node.
fn differ(a: &Node, b: &Node, path: &mut Vec<String>) -> bool {
    match (a, b) {
        (Node::Null, Node::Null) => false,
        (Node::Bool(x), Node::Bool(y)) => x != y,
        (Node::Int(x), Node::Int(y)) => x != y,
        (Node::Float(x), Node::Float(y)) => {
            !(x.is_nan() && y.is_nan()) && x.to_bits() != y.to_bits()
        }
        (Node::Str(x), Node::Str(y)) => x != y,
        (Node::Seq(xs), Node::Seq(ys)) => {
            for (index, (x, y)) in xs.iter().zip(ys).enumerate() {
                path.push(index.to_string());
                if differ(x, y, path) {
                    return true;
                }
                path.pop();
            }
            if xs.len() != ys.len() {
                path.push(xs.len().min(ys.len()).to_string());
                return true;
            }
            false
        }
        (Node::Map(xs), Node::Map(ys)) => {
            for ((kx, vx), (ky, vy)) in xs.iter().zip(ys) {
                path.push(key_segment(kx));
                if differ(kx, ky, path) || differ(vx, vy, path) {
                    return true;
                }
                path.pop();
            }
            if xs.len() != ys.len() {
                let longer = if xs.len() > ys.len() { xs } else { ys };
                path.push(key_segment(&longer[xs.len().min(ys.len())].0));
                return true;
            }
            false
        }
        _ => true,
    }
}

/// A node path as text: `/` followed by the segments joined with `/`.
pub(super) fn join_path(path: &[String]) -> String {
    format!("/{}", path.join("/"))
}

/// A map key as one path segment: a scalar as its value, a collection as `?`.
pub(super) fn key_segment(key: &Node) -> String {
    match key {
        Node::Null => "null".to_owned(),
        Node::Bool(value) => value.to_string(),
        Node::Int(value) => value.to_string(),
        Node::Float(value) => value.to_string(),
        Node::Str(text) => text.clone(),
        Node::Seq(_) | Node::Map(_) => "?".to_owned(),
    }
}

pub(crate) struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any YAML value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Node, E> {
        Ok(Node::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Node, E> {
        Ok(Node::Int(value.into()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Node, E> {
        Ok(Node::Int(value.into()))
    }

    fn visit_i128<E: de::Error>(self, value: i128) -> Result<Node, E> {
        Ok(Node::Int(value))
    }

    fn visit_u128<E: de::Error>(self, value: u128) -> Result<Node, E> {
        i128::try_from(value)
            .map(Node::Int)
            .map_err(|_| E::custom(format!("integer {value} is out of range")))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Node, E> {
        Ok(Node::Float(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Node, E> {
        Ok(Node::Str(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Node, E> {
        Ok(Node::Str(value))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        Node::deserialize(deserializer)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        Node::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Node>()? {
            items.push(item);
        }
        Ok(Node::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<Node, Node>()? {
            entries.push((key, value));
        }
        Ok(Node::Map(entries))
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Node::Null => serializer.serialize_unit(),
            Node::Bool(value) => serializer.serialize_bool(*value),
            Node::Int(value) => {
                if let Ok(small) = i64::try_from(*value) {
                    serializer.serialize_i64(small)
                } else if let Ok(large) = u64::try_from(*value) {
                    serializer.serialize_u64(large)
                } else {
                    Err(serde::ser::Error::custom(format!(
                        "integer {value} is outside the 64-bit ranges"
                    )))
                }
            }
            Node::Float(value) => serializer.serialize_f64(*value),
            Node::Str(text) => serializer.serialize_str(text),
            Node::Seq(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Node::Map(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::options;

    fn parse(text: &str) -> Node {
        serde_saphyr::from_str_with_options(text, options()).unwrap()
    }

    #[test]
    fn a_map_keeps_file_order_and_non_string_keys() {
        let node = parse("z: 1\na: 2\n7: x\n[k]: y\n~: n\n");
        let Node::Map(entries) = &node else {
            panic!("{node:?}")
        };
        let keys: Vec<String> = entries.iter().map(|(key, _)| key_segment(key)).collect();
        assert_eq!(keys, ["z", "a", "7", "?", "null"]);
        assert!(matches!(entries[3].0, Node::Seq(_)));
    }

    #[test]
    fn integers_read_exactly_at_the_64_bit_edges() {
        let node =
            parse("a: 9223372036854775807\nb: 18446744073709551615\nc: -9223372036854775808\n");
        let ints: Vec<i128> = match &node {
            Node::Map(entries) => entries
                .iter()
                .map(|(_, value)| match value {
                    Node::Int(n) => *n,
                    other => panic!("{other:?}"),
                })
                .collect(),
            other => panic!("{other:?}"),
        };
        assert_eq!(ints, [i64::MAX as i128, u64::MAX as i128, i64::MIN as i128]);
    }

    #[test]
    fn same_compares_floats_by_bits_and_any_nan_equals_any_nan() {
        assert!(Node::Float(f64::NAN).same(&Node::Float(-f64::NAN)));
        assert!(parse("a: .nan").same(&parse("a: -.nan")));
        assert!(!Node::Float(0.0).same(&Node::Float(-0.0)));
        assert!(Node::Float(-0.0).same(&Node::Float(-0.0)));
        assert!(!Node::Float(f64::NAN).same(&Node::Float(1.0)));
        assert!(!Node::Int(1).same(&Node::Float(1.0)));
        assert!(!Node::Str("1".to_owned()).same(&Node::Int(1)));
    }

    #[test]
    fn same_is_order_sensitive_and_first_difference_names_the_node() {
        assert!(!parse("a: 1\nb: 2").same(&parse("b: 2\na: 1")));
        assert_eq!(
            parse("a: 1\nb: [x, y]").first_difference(&parse("a: 1\nb: [x, z]")),
            Some("/b/1".to_owned())
        );
        assert_eq!(
            parse("a: [x]").first_difference(&parse("a: [x, y]")),
            Some("/a/1".to_owned())
        );
        assert_eq!(
            parse("a: 1").first_difference(&parse("a: 1\nb: 2")),
            Some("/b".to_owned())
        );
        assert_eq!(
            parse("a: 1").first_difference(&parse("b: 1")),
            Some("/a".to_owned())
        );
        assert_eq!(
            parse("1").first_difference(&parse("2")),
            Some("/".to_owned())
        );
        assert_eq!(
            parse("a: {b: [1]}").first_difference(&parse("a: {b: [1]}")),
            None
        );
    }

    #[test]
    fn integers_serialize_by_range() {
        let out = serde_saphyr::to_string(&Node::Seq(vec![
            Node::Int(i64::MIN as i128),
            Node::Int(u64::MAX as i128),
        ]))
        .unwrap();
        assert_eq!(out, "- -9223372036854775808\n- 18446744073709551615\n");
        assert!(serde_saphyr::to_string(&Node::Int(u64::MAX as i128 + 1)).is_err());
    }
}
