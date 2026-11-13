//! Minimal bounded XML tree for WebDAV multistatus bodies. Namespace prefixes
//! are dropped (elements are matched by local name); DTDs and external
//! entities are never expanded.

use crate::error::{CalendarError, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 400_000;

/// An element with its direct text and children.
#[derive(Debug, Default, Clone)]
pub struct Node {
    /// Local element name.
    pub name: String,
    /// Attributes with local names.
    pub attrs: Vec<(String, String)>,
    /// Concatenated text and CDATA directly inside the element.
    pub text: String,
    /// Child elements in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// Attribute value.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// First direct child named `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.name == name)
    }

    /// All direct children named `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |node| node.name == name)
    }

    /// First descendant (depth-first) named `name`.
    pub fn find(&self, name: &str) -> Option<&Node> {
        for node in &self.children {
            if node.name == name {
                return Some(node);
            }
            if let Some(found) = node.find(name) {
                return Some(found);
            }
        }
        None
    }

    /// Trimmed text of the first direct child named `name`.
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|node| node.text.trim())
            .filter(|text| !text.is_empty())
    }
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        other => {
            let number = other.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

fn node_of(start: &quick_xml::events::BytesStart<'_>) -> Node {
    let attrs = start
        .attributes()
        .filter_map(|attribute| {
            let attribute = attribute.ok()?;
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()?;
            Some((
                String::from_utf8_lossy(attribute.key.local_name().as_ref()).into_owned(),
                value.into_owned(),
            ))
        })
        .take(16)
        .collect();
    Node {
        name: String::from_utf8_lossy(start.local_name().as_ref()).into_owned(),
        attrs,
        ..Node::default()
    }
}

/// Parses `text` into its root element.
pub fn parse(text: &str) -> Result<Node> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut count = 0usize;
    loop {
        let event = reader.read_event().map_err(|_| CalendarError::Parse)?;
        match event {
            Event::Start(start) => {
                count += 1;
                if stack.len() >= MAX_DEPTH || count > MAX_NODES {
                    return Err(CalendarError::Limit);
                }
                stack.push(node_of(&start));
            }
            Event::Empty(start) => {
                count += 1;
                if stack.len() >= MAX_DEPTH || count > MAX_NODES {
                    return Err(CalendarError::Limit);
                }
                let node = node_of(&start);
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::End(_) => {
                let node = stack.pop().ok_or(CalendarError::Parse)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::Text(content) => {
                if let Some(node) = stack.last_mut() {
                    let decoded = content.decode().map_err(|_| CalendarError::Parse)?;
                    node.text.push_str(&decoded);
                }
            }
            Event::CData(content) => {
                if let Some(node) = stack.last_mut() {
                    let decoded = content.decode().map_err(|_| CalendarError::Parse)?;
                    node.text.push_str(&decoded);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(node) = stack.last_mut() {
                    let name = reference.decode().map_err(|_| CalendarError::Parse)?;
                    // Unknown entities (custom DTD ones) are dropped.
                    if let Some(ch) = entity(&name) {
                        node.text.push(ch);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(CalendarError::Parse);
    }
    root.ok_or(CalendarError::Parse)
}

/// Escapes text for an XML body.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}
