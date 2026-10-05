//! The window's tabs (docs/29): tab groups laid out as a tree of splits, which
//! group has focus, and the models a code tab keeps for itself. GTK-free, so
//! the layout's rules are tested here; the window builds itself from it. The
//! macOS twins are `EditorLayout` and `Workspace`.
//!
//! The layout is saved in a project's `local.json` in the same JSON the macOS
//! shell writes (Swift's synthesized `Codable`), so a project carries its
//! layout between the two: an enum case is an object keyed by its name, with
//! its payload as `_0`, and ids are upper-case UUIDs.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::audio;
use super::decompile::Decompile;
use super::graph::GraphModel;
use super::graphics as gfx;

// MARK: Ids

/// A tab's, group's or split's id: a UUID, written upper-case as Foundation
/// writes one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(uuid::Uuid);

impl Id {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated().to_string().to_uppercase())
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The first group of digits is enough to tell tabs apart in a test.
        write!(f, "Id({})", &self.to_string()[..8])
    }
}

impl std::str::FromStr for Id {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(s).map(Self)
    }
}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// MARK: What a tab shows

/// How a code tab shows its routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeRep {
    Assembly,
    C,
    Graph,
    Hex,
    Both,
}

impl CodeRep {
    pub const ALL: [CodeRep; 5] = [
        CodeRep::Assembly,
        CodeRep::C,
        CodeRep::Graph,
        CodeRep::Hex,
        CodeRep::Both,
    ];

    /// The name in `local.json` and a content's key.
    pub fn raw(self) -> &'static str {
        match self {
            CodeRep::Assembly => "assembly",
            CodeRep::C => "c",
            CodeRep::Graph => "graph",
            CodeRep::Hex => "hex",
            CodeRep::Both => "both",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.raw() == raw)
    }

    /// The view's name, as the sidebar and a tab say it.
    pub fn view_title(self) -> &'static str {
        match self {
            CodeRep::Assembly => "Disassembly",
            CodeRep::C => "Pseudo-C",
            CodeRep::Graph => "Graph",
            CodeRep::Hex => "Hex",
            CodeRep::Both => "Hex and Disassembly",
        }
    }
}

/// A graphics view's name in `local.json`: the macOS raw value.
fn gfx_raw(t: gfx::Tab) -> &'static str {
    t.id()
}

fn gfx_from_raw(raw: &str) -> Option<gfx::Tab> {
    gfx::Tab::ALL.into_iter().find(|t| gfx_raw(*t) == raw)
}

/// A sound view's name in `local.json`: the macOS raw value, which for Audio
/// RAM is `aram` where the shell's own id is `audio-ram`.
fn audio_raw(t: audio::Tab) -> &'static str {
    match t {
        audio::Tab::Aram => "aram",
        t => t.id(),
    }
}

fn audio_from_raw(raw: &str) -> Option<audio::Tab> {
    audio::Tab::ALL.into_iter().find(|t| audio_raw(*t) == raw)
}

/// What a tab shows. Code may be open in several tabs; everything else has at
/// most one tab in a window, because the recording, the player and the
/// comparison behind it are the document's (docs/29, scope decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorContent {
    Code(CodeRep),
    /// The header and vectors as a tab, taken out of the sidebar on 2 October
    /// 2026. Kept so a layout saved with one still reads; reopening leaves it
    /// out.
    Header,
    Atlas,
    Compare,
    Source,
    Graphics(gfx::Tab),
    Audio(audio::Tab),
    Tutor,
}

impl EditorContent {
    /// Whether a window may hold only one tab of this.
    pub fn is_singleton(self) -> bool {
        !matches!(self, EditorContent::Code(_))
    }

    /// Two contents with the same key are the same view for the
    /// one-tab-per-window rule.
    pub fn singleton_key(self) -> String {
        match self {
            EditorContent::Code(r) => format!("code.{}", r.raw()),
            EditorContent::Header => "header".into(),
            EditorContent::Atlas => "atlas".into(),
            EditorContent::Compare => "compare".into(),
            EditorContent::Source => "source".into(),
            EditorContent::Graphics(t) => format!("graphics.{}", gfx_raw(t)),
            EditorContent::Audio(t) => format!("audio.{}", audio_raw(t)),
            EditorContent::Tutor => "tutor".into(),
        }
    }

    /// The view's name, as the sidebar and a tab say it.
    pub fn title(self) -> &'static str {
        match self {
            EditorContent::Code(r) => r.view_title(),
            EditorContent::Header => "Header and Vectors",
            EditorContent::Atlas => "Atlas",
            EditorContent::Compare => "Compare",
            EditorContent::Source => "Source",
            EditorContent::Graphics(t) => t.title(),
            EditorContent::Audio(t) => t.title(),
            EditorContent::Tutor => "Tutor",
        }
    }
}

/// Swift's encoding of `EditorContent`: `{"code":{"_0":"assembly"}}`,
/// `{"atlas":{}}`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ContentJson {
    Code {
        #[serde(rename = "_0")]
        rep: CodeRep,
    },
    Header {},
    Atlas {},
    Compare {},
    Source {},
    Graphics {
        #[serde(rename = "_0")]
        view: String,
    },
    Audio {
        #[serde(rename = "_0")]
        view: String,
    },
    Tutor {},
}

impl Serialize for EditorContent {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match *self {
            EditorContent::Code(rep) => ContentJson::Code { rep },
            EditorContent::Header => ContentJson::Header {},
            EditorContent::Atlas => ContentJson::Atlas {},
            EditorContent::Compare => ContentJson::Compare {},
            EditorContent::Source => ContentJson::Source {},
            EditorContent::Graphics(t) => ContentJson::Graphics {
                view: gfx_raw(t).into(),
            },
            EditorContent::Audio(t) => ContentJson::Audio {
                view: audio_raw(t).into(),
            },
            EditorContent::Tutor => ContentJson::Tutor {},
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for EditorContent {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let unknown = |v: &str| serde::de::Error::custom(format!("no view named {v}"));
        Ok(match ContentJson::deserialize(d)? {
            ContentJson::Code { rep } => EditorContent::Code(rep),
            ContentJson::Header {} => EditorContent::Header,
            ContentJson::Atlas {} => EditorContent::Atlas,
            ContentJson::Compare {} => EditorContent::Compare,
            ContentJson::Source {} => EditorContent::Source,
            ContentJson::Graphics { view } => {
                EditorContent::Graphics(gfx_from_raw(&view).ok_or_else(|| unknown(&view))?)
            }
            ContentJson::Audio { view } => {
                EditorContent::Audio(audio_from_raw(&view).ok_or_else(|| unknown(&view))?)
            }
            ContentJson::Tutor {} => EditorContent::Tutor,
        })
    }
}

// MARK: The tree

/// One tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorItem {
    pub id: Id,
    pub content: EditorContent,
    /// Whether the tab scrolls to the selection when another tab moves it.
    pub follows_selection: bool,
}

impl EditorItem {
    pub fn new(content: EditorContent) -> Self {
        Self {
            id: Id::new(),
            content,
            follows_selection: true,
        }
    }
}

/// A group of tabs, one of them shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: Id,
    pub items: Vec<EditorItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Id>,
}

impl TabGroup {
    /// A new group showing its first tab.
    pub fn new(items: Vec<EditorItem>) -> Self {
        Self {
            id: Id::new(),
            selected: items.first().map(|i| i.id),
            items,
        }
    }

    pub fn selected_item(&self) -> Option<&EditorItem> {
        self.items.iter().find(|i| Some(i.id) == self.selected)
    }
}

/// `Horizontal` lays children side by side, `Vertical` stacks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSplit {
    pub id: Id,
    pub axis: SplitAxis,
    pub children: Vec<LayoutNode>,
    /// One per child, summing to 1.
    pub fractions: Vec<f64>,
}

impl LayoutSplit {
    /// A split sharing its room equally.
    pub fn new(axis: SplitAxis, children: Vec<LayoutNode>) -> Self {
        let n = children.len().max(1);
        Self {
            id: Id::new(),
            axis,
            fractions: vec![1.0 / n as f64; children.len()],
            children,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutNode {
    Group(TabGroup),
    Split(LayoutSplit),
}

/// Swift's encoding of `LayoutNode`: `{"group":{"_0":{…}}}`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum NodeJson {
    Group {
        #[serde(rename = "_0")]
        group: TabGroup,
    },
    Split {
        #[serde(rename = "_0")]
        split: LayoutSplit,
    },
}

impl Serialize for LayoutNode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            LayoutNode::Group(g) => NodeJson::Group { group: g.clone() },
            LayoutNode::Split(sp) => NodeJson::Split { split: sp.clone() },
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for LayoutNode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match NodeJson::deserialize(d)? {
            NodeJson::Group { group } => LayoutNode::Group(group),
            NodeJson::Split { split } => LayoutNode::Split(split),
        })
    }
}

// MARK: Dropping

/// Where a tab dropped on a group goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropZone {
    Center,
    Edge(DropEdge),
}

/// A rectangle whose y grows downward, as GTK's do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a point in a group's content falls: the outer third of a side splits
/// there, the middle moves the tab into the group, and in a corner the nearer
/// edge, measured as a share of the group's width or height, wins.
pub fn drop_zone(x: f64, y: f64, rect: Rect) -> DropZone {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return DropZone::Center;
    }
    let distances = [
        (DropEdge::Left, (x - rect.x) / rect.width),
        (DropEdge::Right, (rect.x + rect.width - x) / rect.width),
        (DropEdge::Top, (y - rect.y) / rect.height),
        (DropEdge::Bottom, (rect.y + rect.height - y) / rect.height),
    ];
    let (edge, share) = distances
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("four edges");
    if share < 1.0 / 3.0 {
        DropZone::Edge(edge)
    } else {
        DropZone::Center
    }
}

/// What a drag into the editor area carries: a tab, or something to open (a
/// row of the sidebar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabDrop {
    Item(Id),
    Open(EditorContent),
    /// A new code tab at a SNES address: a label dragged from the sidebar.
    OpenAt(CodeRep, u32),
}

/// The drag's type, as macOS names its pasteboard type.
pub const TAB_DROP_TYPE: &str = "io.github.ayodehi.romlens.tab";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum DropJson {
    Item {
        #[serde(rename = "_0")]
        id: Id,
    },
    Open {
        #[serde(rename = "_0")]
        content: EditorContent,
    },
    OpenAt {
        #[serde(rename = "_0")]
        rep: CodeRep,
        address: u32,
    },
}

impl TabDrop {
    pub fn to_json(self) -> String {
        serde_json::to_string(&match self {
            TabDrop::Item(id) => DropJson::Item { id },
            TabDrop::Open(content) => DropJson::Open { content },
            TabDrop::OpenAt(rep, address) => DropJson::OpenAt { rep, address },
        })
        .expect("a drop serializes")
    }

    pub fn from_json(s: &str) -> Option<Self> {
        Some(match serde_json::from_str(s).ok()? {
            DropJson::Item { id } => TabDrop::Item(id),
            DropJson::Open { content } => TabDrop::Open(content),
            DropJson::OpenAt { rep, address } => TabDrop::OpenAt(rep, address),
        })
    }
}

/// Where a drop lands in a group: between two tabs of its bar, or in a zone
/// of its view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    TabBar(usize),
    Zone(DropZone),
}

/// The four arrangements in View › Editor Layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPreset {
    Single,
    TwoColumns,
    TwoRows,
    Three,
}

impl LayoutPreset {
    pub const ALL: [LayoutPreset; 4] = [
        LayoutPreset::Single,
        LayoutPreset::TwoColumns,
        LayoutPreset::TwoRows,
        LayoutPreset::Three,
    ];

    pub fn title(self) -> &'static str {
        match self {
            LayoutPreset::Single => "Single",
            LayoutPreset::TwoColumns => "Two Columns",
            LayoutPreset::TwoRows => "Two Rows",
            LayoutPreset::Three => "Three",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            LayoutPreset::Single => "single",
            LayoutPreset::TwoColumns => "two-columns",
            LayoutPreset::TwoRows => "two-rows",
            LayoutPreset::Three => "three",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    fn groups(self) -> usize {
        match self {
            LayoutPreset::Single => 1,
            LayoutPreset::TwoColumns | LayoutPreset::TwoRows => 2,
            LayoutPreset::Three => 3,
        }
    }
}

// MARK: The layout

/// The editor area: tab groups laid out as a tree of splits.
///
/// Every change is a method here, so the drop handler, the menus and the
/// tests all go through the same code. A group emptied by moving or closing
/// its last tab goes, unless it is the only one; a split left with one child
/// is replaced by it; and a split inside another along the same axis is merged
/// into it, so the tree stays as flat as the arrangement on screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorLayout {
    pub root: LayoutNode,
}

impl EditorLayout {
    /// One group holding `items`.
    pub fn single(items: Vec<EditorItem>) -> Self {
        Self {
            root: LayoutNode::Group(TabGroup::new(items)),
        }
    }

    // MARK: Reading

    /// Every group, left to right and top to bottom.
    pub fn groups(&self) -> Vec<&TabGroup> {
        fn walk<'a>(n: &'a LayoutNode, out: &mut Vec<&'a TabGroup>) {
            match n {
                LayoutNode::Group(g) => out.push(g),
                LayoutNode::Split(s) => s.children.iter().for_each(|c| walk(c, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    pub fn items(&self) -> Vec<EditorItem> {
        self.groups()
            .iter()
            .flat_map(|g| g.items.iter().copied())
            .collect()
    }

    pub fn group(&self, id: Id) -> Option<&TabGroup> {
        self.groups().into_iter().find(|g| g.id == id)
    }

    pub fn group_containing(&self, item: Id) -> Option<&TabGroup> {
        self.groups()
            .into_iter()
            .find(|g| g.items.iter().any(|i| i.id == item))
    }

    pub fn item(&self, id: Id) -> Option<EditorItem> {
        self.items().into_iter().find(|i| i.id == id)
    }

    /// The tab already showing a view that may have only one.
    pub fn existing(&self, content: EditorContent) -> Option<EditorItem> {
        if !content.is_singleton() {
            return None;
        }
        let key = content.singleton_key();
        self.items()
            .into_iter()
            .find(|i| i.content.singleton_key() == key)
    }

    pub fn split_node(&self, id: Id) -> Option<&LayoutSplit> {
        fn find(n: &LayoutNode, id: Id) -> Option<&LayoutSplit> {
            let LayoutNode::Split(s) = n else { return None };
            if s.id == id {
                return Some(s);
            }
            s.children.iter().find_map(|c| find(c, id))
        }
        find(&self.root, id)
    }

    // MARK: Changing

    /// Opens `content` in `group`, after its shown tab, and shows it. A view
    /// that may have only one tab and already has one is shown where it is
    /// instead. Returns the tab's id, or `None` if `group` does not exist.
    pub fn open(&mut self, content: EditorContent, group: Id, follows: bool) -> Option<Id> {
        if let Some(found) = self.existing(content) {
            self.select(found.id);
            return Some(found.id);
        }
        let item = EditorItem {
            follows_selection: follows,
            ..EditorItem::new(content)
        };
        let mut placed = false;
        self.update_group(group, |g| {
            let at = g
                .items
                .iter()
                .position(|i| Some(i.id) == g.selected)
                .map_or(g.items.len(), |i| i + 1);
            g.items.insert(at, item);
            g.selected = Some(item.id);
            placed = true;
        });
        placed.then_some(item.id)
    }

    /// Shows `item` in its group.
    pub fn select(&mut self, item: Id) {
        let Some(g) = self.group_containing(item).map(|g| g.id) else {
            return;
        };
        self.update_group(g, |g| g.selected = Some(item));
    }

    /// Moves `item` into `group` at `index` (the end if `None`) and shows it.
    /// Within one group this reorders.
    pub fn move_item(&mut self, item_id: Id, group: Id, index: Option<usize>) {
        let (Some(source), Some(item)) = (
            self.group_containing(item_id).map(|g| g.id),
            self.item(item_id),
        ) else {
            return;
        };
        if self.group(group).is_none() {
            return;
        }
        if source == group {
            self.update_group(group, |g| {
                let Some(from) = g.items.iter().position(|i| i.id == item_id) else {
                    return;
                };
                g.items.remove(from);
                let mut to = index.unwrap_or(g.items.len() + 1);
                if to > from {
                    to -= 1;
                }
                let to = to.min(g.items.len());
                g.items.insert(to, item);
                g.selected = Some(item_id);
            });
            return;
        }
        self.take(item_id);
        self.update_group(group, |g| {
            let at = index.unwrap_or(g.items.len()).min(g.items.len());
            g.items.insert(at, item);
            g.selected = Some(item_id);
        });
        self.remove_if_empty(source);
    }

    /// Takes `item` out of where it is and puts it in a new group on `edge` of
    /// `group`. Returns the new group's id, or `None` when there is nothing to
    /// split (the item is the target group's only tab).
    pub fn split(&mut self, group: Id, edge: DropEdge, item_id: Id) -> Option<Id> {
        let source = self.group_containing(item_id)?;
        let (source_id, alone) = (source.id, source.items.len() == 1);
        let item = self.item(item_id)?;
        self.group(group)?;
        if source_id == group && alone {
            return None;
        }
        self.take(item_id);
        let fresh = TabGroup::new(vec![item]);
        let fresh_id = fresh.id;
        let axis = match edge {
            DropEdge::Left | DropEdge::Right => SplitAxis::Horizontal,
            DropEdge::Top | DropEdge::Bottom => SplitAxis::Vertical,
        };
        let before = matches!(edge, DropEdge::Left | DropEdge::Top);
        let root = std::mem::replace(&mut self.root, LayoutNode::Group(TabGroup::new(vec![])));
        self.root = insert(LayoutNode::Group(fresh), group, axis, before, root);
        if source_id != group {
            self.remove_if_empty(source_id);
        }
        self.normalize();
        Some(fresh_id)
    }

    /// Closes `item`. Its group goes when emptied, unless it is the last.
    pub fn close(&mut self, item: Id) {
        let Some(g) = self.group_containing(item).map(|g| g.id) else {
            return;
        };
        self.take(item);
        self.remove_if_empty(g);
    }

    /// Makes a split's children equal.
    pub fn equalize(&mut self, split: Id) {
        self.update_split(split, |s| {
            let n = s.children.len();
            s.fractions = vec![1.0 / n as f64; n];
        });
    }

    /// Sets a split's fractions, as a divider drag does; they are scaled to
    /// sum to 1, and a list of the wrong length is ignored.
    pub fn set_fractions(&mut self, split: Id, fractions: &[f64]) {
        self.update_split(split, |s| {
            let total: f64 = fractions.iter().sum();
            if fractions.len() != s.children.len() || total <= 0.0 {
                return;
            }
            s.fractions = fractions.iter().map(|f| f / total).collect();
        });
    }

    /// Rearranges into a preset, keeping every tab: groups beyond those the
    /// preset has are merged into its last, and a preset with more groups than
    /// there are gets empty ones.
    pub fn apply(&mut self, preset: LayoutPreset) {
        let count = preset.groups();
        let mut gs: Vec<TabGroup> = self.groups().into_iter().cloned().collect();
        while gs.len() < count {
            gs.push(TabGroup::new(vec![]));
        }
        if gs.len() > count {
            let extra: Vec<TabGroup> = gs.drain(count..).collect();
            let last = &mut gs[count - 1];
            for g in extra {
                last.items.extend(g.items);
            }
            if last.selected.is_none() {
                last.selected = last.items.first().map(|i| i.id);
            }
        }
        let mut gs = gs.into_iter().map(LayoutNode::Group);
        let mut next = || gs.next().expect("as many groups as the preset has");
        self.root = match preset {
            LayoutPreset::Single => next(),
            LayoutPreset::TwoColumns => LayoutNode::Split(LayoutSplit::new(
                SplitAxis::Horizontal,
                vec![next(), next()],
            )),
            LayoutPreset::TwoRows => {
                LayoutNode::Split(LayoutSplit::new(SplitAxis::Vertical, vec![next(), next()]))
            }
            LayoutPreset::Three => {
                let left = next();
                let right = LayoutSplit::new(SplitAxis::Vertical, vec![next(), next()]);
                LayoutNode::Split(LayoutSplit::new(
                    SplitAxis::Horizontal,
                    vec![left, LayoutNode::Split(right)],
                ))
            }
        };
    }

    /// Changes the tab `id` wherever it is.
    pub fn update_item(&mut self, id: Id, change: impl FnOnce(&mut EditorItem)) {
        let Some(g) = self.group_containing(id).map(|g| g.id) else {
            return;
        };
        self.update_group(g, |g| {
            if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                change(i);
            }
        });
    }

    // MARK: Helpers

    fn take(&mut self, item: Id) {
        let Some(g) = self.group_containing(item).map(|g| g.id) else {
            return;
        };
        self.update_group(g, |g| {
            let Some(i) = g.items.iter().position(|x| x.id == item) else {
                return;
            };
            g.items.remove(i);
            if g.selected == Some(item) {
                // The neighbour on the right, else the left, as tab bars do.
                g.selected = (!g.items.is_empty()).then(|| g.items[i.min(g.items.len() - 1)].id);
            }
        });
    }

    fn remove_if_empty(&mut self, group: Id) {
        let empty = self.group(group).is_some_and(|g| g.items.is_empty());
        if !empty || self.groups().len() <= 1 {
            return;
        }
        let root = std::mem::replace(&mut self.root, LayoutNode::Group(TabGroup::new(vec![])));
        self.root = remove(group, root.clone()).unwrap_or(root);
        self.normalize();
    }

    fn update_group(&mut self, id: Id, change: impl FnOnce(&mut TabGroup)) {
        fn walk(n: &mut LayoutNode, id: Id, change: &mut Option<impl FnOnce(&mut TabGroup)>) {
            match n {
                LayoutNode::Group(g) if g.id == id => {
                    if let Some(c) = change.take() {
                        c(g);
                    }
                }
                LayoutNode::Group(_) => {}
                LayoutNode::Split(s) => s.children.iter_mut().for_each(|c| walk(c, id, change)),
            }
        }
        walk(&mut self.root, id, &mut Some(change));
    }

    fn update_split(&mut self, id: Id, change: impl FnOnce(&mut LayoutSplit)) {
        fn walk(n: &mut LayoutNode, id: Id, change: &mut Option<impl FnOnce(&mut LayoutSplit)>) {
            let LayoutNode::Split(s) = n else { return };
            s.children.iter_mut().for_each(|c| walk(c, id, change));
            if s.id == id
                && let Some(c) = change.take()
            {
                c(s);
            }
        }
        walk(&mut self.root, id, &mut Some(change));
    }

    /// Splits with one child become the child; a split inside one along the
    /// same axis is merged into it, its fraction shared among its children.
    fn normalize(&mut self) {
        fn walk(n: LayoutNode) -> LayoutNode {
            let LayoutNode::Split(mut s) = n else {
                return n;
            };
            let mut children = Vec::new();
            let mut fractions = Vec::new();
            let old = std::mem::take(&mut s.children);
            for (c, f) in old.into_iter().map(walk).zip(s.fractions.iter().copied()) {
                match c {
                    LayoutNode::Split(inner) if inner.axis == s.axis => {
                        fractions.extend(inner.fractions.iter().map(|x| x * f));
                        children.extend(inner.children);
                    }
                    c => {
                        children.push(c);
                        fractions.push(f);
                    }
                }
            }
            if children.len() == 1 {
                return children.pop().expect("one child");
            }
            s.children = children;
            s.fractions = fractions;
            LayoutNode::Split(s)
        }
        let root = std::mem::replace(&mut self.root, LayoutNode::Group(TabGroup::new(vec![])));
        self.root = walk(root);
    }
}

/// `node` with the group removed; `None` when the node was that group.
fn remove(group: Id, node: LayoutNode) -> Option<LayoutNode> {
    match node {
        LayoutNode::Group(g) => (g.id != group).then_some(LayoutNode::Group(g)),
        LayoutNode::Split(mut s) => {
            let mut children = Vec::new();
            let mut fractions = Vec::new();
            for (c, f) in std::mem::take(&mut s.children)
                .into_iter()
                .zip(s.fractions.iter().copied())
            {
                if let Some(kept) = remove(group, c) {
                    children.push(kept);
                    fractions.push(f);
                }
            }
            if children.is_empty() {
                return None;
            }
            let total: f64 = fractions.iter().sum();
            let n = children.len() as f64;
            s.fractions = fractions
                .iter()
                .map(|f| if total > 0.0 { f / total } else { 1.0 / n })
                .collect();
            s.children = children;
            Some(LayoutNode::Split(s))
        }
    }
}

/// `node` with `new` placed beside the group `target`: as a sibling when the
/// group's parent splits along `axis`, which shares out the target's fraction
/// between the two, or else in a new split in its place.
fn insert(
    new: LayoutNode,
    target: Id,
    axis: SplitAxis,
    before: bool,
    node: LayoutNode,
) -> LayoutNode {
    let mut new = Some(new);
    fn walk(
        new: &mut Option<LayoutNode>,
        target: Id,
        axis: SplitAxis,
        before: bool,
        node: LayoutNode,
    ) -> LayoutNode {
        match node {
            LayoutNode::Group(g) if g.id == target => {
                let Some(fresh) = new.take() else {
                    return LayoutNode::Group(g);
                };
                let pair = if before {
                    vec![fresh, LayoutNode::Group(g)]
                } else {
                    vec![LayoutNode::Group(g), fresh]
                };
                LayoutNode::Split(LayoutSplit::new(axis, pair))
            }
            LayoutNode::Group(g) => LayoutNode::Group(g),
            LayoutNode::Split(mut s) => {
                let i = s
                    .children
                    .iter()
                    .position(|c| matches!(c, LayoutNode::Group(g) if g.id == target));
                if s.axis == axis
                    && let Some(i) = i
                    && let Some(fresh) = new.take()
                {
                    let half = s.fractions[i] / 2.0;
                    s.fractions[i] = half;
                    let at = if before { i } else { i + 1 };
                    s.children.insert(at, fresh);
                    s.fractions.insert(at, half);
                    return LayoutNode::Split(s);
                }
                s.children = std::mem::take(&mut s.children)
                    .into_iter()
                    .map(|c| walk(new, target, axis, before, c))
                    .collect();
                LayoutNode::Split(s)
            }
        }
    }
    walk(&mut new, target, axis, before, node)
}

// MARK: The workspace

/// The window's tabs: the layout, which group has focus, and the models a code
/// tab keeps for itself. Every change to the layout from the window goes
/// through here, so the focus always names a group that exists. The selection
/// stays on the document, shared by every tab.
pub struct Workspace {
    layout: EditorLayout,
    focused_group: Id,
    /// The code tab that last had focus: what the C and Graph models of "the"
    /// editor mean when no tab is named.
    last_code_item: Option<Id>,
    decompilers: HashMap<Id, Decompile>,
    graphs: HashMap<Id, GraphModel>,
    /// For a call that names no code tab while none exists.
    fallback: Id,
}

impl Default for Workspace {
    /// One group holding a Hex tab, as a new window opens.
    fn default() -> Self {
        Self::new(EditorLayout::single(vec![EditorItem::new(
            EditorContent::Code(CodeRep::Hex),
        )]))
    }
}

impl Workspace {
    pub fn new(layout: EditorLayout) -> Self {
        let focused_group = layout.groups()[0].id;
        let last_code_item = layout
            .items()
            .iter()
            .find(|i| matches!(i.content, EditorContent::Code(_)))
            .map(|i| i.id);
        Self {
            layout,
            focused_group,
            last_code_item,
            decompilers: HashMap::new(),
            graphs: HashMap::new(),
            fallback: Id::new(),
        }
    }

    // MARK: Reading

    pub fn layout(&self) -> &EditorLayout {
        &self.layout
    }

    pub fn focused_group(&self) -> Id {
        self.focused_group
    }

    pub fn focused_item(&self) -> Option<EditorItem> {
        self.layout
            .group(self.focused_group)
            .and_then(|g| g.selected_item().copied())
    }

    /// The tab shown in each group.
    pub fn visible_items(&self) -> Vec<EditorItem> {
        self.layout
            .groups()
            .iter()
            .filter_map(|g| g.selected_item().copied())
            .collect()
    }

    /// The code tab C and Graph act on when no tab is named.
    pub fn current_code_item(&self) -> Id {
        if let Some(f) = self.focused_item()
            && matches!(f.content, EditorContent::Code(_))
        {
            return f.id;
        }
        match self.last_code_item {
            Some(last) if self.layout.item(last).is_some() => last,
            _ => self.fallback,
        }
    }

    /// The tab's own decompiler (the current code tab's for `None`), made when
    /// first wanted.
    pub fn decompiler(&mut self, id: Option<Id>) -> &mut Decompile {
        let key = id.unwrap_or_else(|| self.current_code_item());
        self.decompilers.entry(key).or_default()
    }

    pub fn graph(&mut self, id: Option<Id>) -> &mut GraphModel {
        let key = id.unwrap_or_else(|| self.current_code_item());
        self.graphs.entry(key).or_default()
    }

    /// The analysis changed: every tab's C and graph are stale.
    pub fn invalidate_all(&mut self) {
        self.decompilers
            .values_mut()
            .for_each(Decompile::invalidate);
        self.graphs.values_mut().for_each(GraphModel::invalidate);
    }

    // MARK: Changing

    /// Opens `content` in `group` (the focused one if `None`), or shows its
    /// existing tab, and gives that tab focus. A second code tab showing the
    /// same representation as an open one does not follow the selection, since
    /// two tabs that always scroll together are one tab twice.
    pub fn open(&mut self, content: EditorContent, group: Option<Id>) -> Option<Id> {
        let target = group.unwrap_or(self.focused_group);
        let twin = matches!(content, EditorContent::Code(_))
            && self.layout.items().iter().any(|i| i.content == content);
        let id = self.layout.open(content, target, !twin)?;
        self.focus_item(id);
        Some(id)
    }

    /// Shows `item` in its group and gives the group focus.
    pub fn focus_item(&mut self, id: Id) {
        let Some(g) = self.layout.group_containing(id).map(|g| g.id) else {
            return;
        };
        self.layout.select(id);
        self.focused_group = g;
        self.note_focus();
    }

    pub fn focus_group(&mut self, id: Id) {
        if self.layout.group(id).is_none() {
            return;
        }
        self.focused_group = id;
        self.note_focus();
    }

    pub fn set_follows_selection(&mut self, id: Id, follows: bool) {
        self.change(|l| l.update_item(id, |i| i.follows_selection = follows));
    }

    pub fn close(&mut self, id: Id) {
        let group = self.layout.group_containing(id).map(|g| g.id);
        self.change(|l| l.close(id));
        self.decompilers.remove(&id);
        self.graphs.remove(&id);
        if let Some(g) = group
            && self.layout.group(g).is_some()
        {
            self.focused_group = g;
        }
        self.repair_focus();
    }

    pub fn move_item(&mut self, id: Id, group: Id, index: Option<usize>) {
        self.change(|l| l.move_item(id, group, index));
        self.focus_item(id);
    }

    pub fn split(&mut self, group: Id, edge: DropEdge, id: Id) -> Option<Id> {
        let mut fresh = None;
        self.change(|l| fresh = l.split(group, edge, id));
        if let Some(f) = fresh {
            self.focus_group(f);
        }
        fresh
    }

    pub fn apply(&mut self, preset: LayoutPreset) {
        self.change(|l| l.apply(preset));
    }

    pub fn equalize(&mut self, split: Id) {
        self.change(|l| l.equalize(split));
    }

    pub fn set_fractions(&mut self, split: Id, fractions: &[f64]) {
        self.change(|l| l.set_fractions(split, fractions));
    }

    /// Replaces the whole layout, as reopening a project does. Tabs that are
    /// gone take their models with them.
    pub fn restore(&mut self, saved: EditorLayout, focus: Option<Id>) {
        let ids: Vec<Id> = saved.items().iter().map(|i| i.id).collect();
        self.decompilers.retain(|k, _| ids.contains(k));
        self.graphs.retain(|k, _| ids.contains(k));
        self.focused_group = focus
            .and_then(|f| saved.group(f).map(|g| g.id))
            .unwrap_or_else(|| saved.groups()[0].id);
        self.layout = saved;
        self.note_focus();
    }

    // MARK: Helpers

    fn change(&mut self, body: impl FnOnce(&mut EditorLayout)) {
        body(&mut self.layout);
        self.repair_focus();
    }

    fn repair_focus(&mut self) {
        if self.layout.group(self.focused_group).is_none() {
            self.focused_group = self.layout.groups()[0].id;
        }
        self.note_focus();
    }

    fn note_focus(&mut self) {
        if let Some(f) = self.focused_item()
            && matches!(f.content, EditorContent::Code(_))
        {
            self.last_code_item = Some(f.id);
        }
    }
}

/// What `local.json` keeps of the window: the tabs, which group has focus,
/// which panels show and the drawer's tab. The four flags are always written,
/// since the macOS shell needs every one of them to read the record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub layout: EditorLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_group: Option<Id>,
    #[serde(default = "yes")]
    pub sidebar: bool,
    #[serde(default = "yes")]
    pub inspector: bool,
    #[serde(default = "yes")]
    pub strip: bool,
    #[serde(default)]
    pub tutor_in_drawer: bool,
}

fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> EditorItem {
        EditorItem::new(EditorContent::Code(CodeRep::Assembly))
    }

    /// The shape of the tree, for comparing: groups as their tab counts, a
    /// split as its axis and children.
    fn shape(n: &LayoutNode) -> String {
        match n {
            LayoutNode::Group(g) => g.items.len().to_string(),
            LayoutNode::Split(s) => format!(
                "{}({})",
                if s.axis == SplitAxis::Horizontal {
                    "H"
                } else {
                    "V"
                },
                s.children.iter().map(shape).collect::<Vec<_>>().join(",")
            ),
        }
    }

    fn ids(items: &[EditorItem]) -> Vec<Id> {
        items.iter().map(|i| i.id).collect()
    }

    fn first_group(l: &EditorLayout) -> Id {
        l.groups()[0].id
    }

    #[test]
    fn opening_puts_the_tab_after_the_shown_one_and_shows_it() {
        let (a, b) = (code(), code());
        let mut l = EditorLayout::single(vec![a, b]);
        let g = first_group(&l);
        let c = l
            .open(EditorContent::Code(CodeRep::C), g, true)
            .expect("opened");
        assert_eq!(ids(&l.groups()[0].items), [a.id, c, b.id]);
        assert_eq!(l.groups()[0].selected, Some(c));
    }

    #[test]
    fn a_view_with_one_tab_is_shown_where_it_is_instead_of_opened_again() {
        let mut l = EditorLayout::single(vec![code()]);
        let g = first_group(&l);
        let atlas = l.open(EditorContent::Atlas, g, true).unwrap();
        l.open(EditorContent::Code(CodeRep::Hex), g, true);
        assert_eq!(l.open(EditorContent::Atlas, g, true), Some(atlas));
        let atlases = l
            .items()
            .iter()
            .filter(|i| i.content == EditorContent::Atlas)
            .count();
        assert_eq!(atlases, 1);
        assert_eq!(l.groups()[0].selected, Some(atlas));
        // Code is not limited: two assembly tabs.
        l.open(EditorContent::Code(CodeRep::Assembly), g, true);
        let assembly = l
            .items()
            .iter()
            .filter(|i| i.content == EditorContent::Code(CodeRep::Assembly))
            .count();
        assert_eq!(assembly, 2);
    }

    #[test]
    fn splitting_puts_the_tab_in_a_new_group_on_that_side() {
        let (a, b) = (code(), code());
        let mut l = EditorLayout::single(vec![a, b]);
        let g = first_group(&l);
        let fresh = l.split(g, DropEdge::Right, b.id).unwrap();
        assert_eq!(shape(&l.root), "H(1,1)");
        let order: Vec<Id> = l.groups().iter().map(|g| g.id).collect();
        assert_eq!(order, [g, fresh]);
        assert_eq!(ids(&l.group(fresh).unwrap().items), [b.id]);
        // Moving the left group's only tab above the right one empties the
        // left group, which goes.
        l.split(fresh, DropEdge::Top, a.id);
        assert_eq!(shape(&l.root), "V(1,1)");
        assert_eq!(l.groups().last().unwrap().id, fresh);
    }

    #[test]
    fn each_edge_places_the_group_on_its_side() {
        for (edge, expected) in [
            (DropEdge::Left, "H"),
            (DropEdge::Right, "H"),
            (DropEdge::Top, "V"),
            (DropEdge::Bottom, "V"),
        ] {
            let (a, b) = (code(), code());
            let mut l = EditorLayout::single(vec![a, b]);
            let target = first_group(&l);
            let fresh = l.split(target, edge, b.id).unwrap();
            assert_eq!(shape(&l.root), format!("{expected}(1,1)"));
            let order: Vec<Id> = l.groups().iter().map(|g| g.id).collect();
            let before = matches!(edge, DropEdge::Left | DropEdge::Top);
            assert_eq!(
                order,
                if before {
                    [fresh, target]
                } else {
                    [target, fresh]
                }
            );
        }
    }

    #[test]
    fn splitting_a_group_with_its_only_tab_does_nothing() {
        let a = code();
        let mut l = EditorLayout::single(vec![a]);
        let g = first_group(&l);
        assert_eq!(l.split(g, DropEdge::Right, a.id), None);
        assert_eq!(shape(&l.root), "1");
    }

    #[test]
    fn a_split_along_the_same_axis_adds_a_sibling_and_shares_the_fraction() {
        let (a, b, c) = (code(), code(), code());
        let mut l = EditorLayout::single(vec![a, b, c]);
        let g = first_group(&l);
        let right = l.split(g, DropEdge::Right, b.id).unwrap();
        l.split(right, DropEdge::Right, c.id);
        assert_eq!(shape(&l.root), "H(1,1,1)");
        let LayoutNode::Split(s) = &l.root else {
            panic!("not a split")
        };
        assert_eq!(s.fractions, [0.5, 0.25, 0.25]);
    }

    #[test]
    fn moving_the_last_tab_out_closes_its_group_and_collapses_the_split() {
        let (a, b) = (code(), code());
        let mut l = EditorLayout::single(vec![a, b]);
        let left = first_group(&l);
        let right = l.split(left, DropEdge::Right, b.id).unwrap();
        l.move_item(b.id, left, None);
        assert_eq!(shape(&l.root), "2");
        assert!(l.group(right).is_none());
        assert_eq!(l.groups()[0].selected, Some(b.id));
    }

    #[test]
    fn moving_within_a_group_reorders() {
        let (a, b, c) = (code(), code(), code());
        let mut l = EditorLayout::single(vec![a, b, c]);
        let g = first_group(&l);
        l.move_item(a.id, g, Some(3));
        assert_eq!(ids(&l.groups()[0].items), [b.id, c.id, a.id]);
        l.move_item(c.id, g, Some(0));
        assert_eq!(ids(&l.groups()[0].items), [c.id, b.id, a.id]);
    }

    #[test]
    fn closing_shows_the_neighbour_and_keeps_the_last_group() {
        let (a, b, c) = (code(), code(), code());
        let mut l = EditorLayout::single(vec![a, b, c]);
        l.select(b.id);
        l.close(b.id);
        assert_eq!(l.groups()[0].selected, Some(c.id));
        l.close(c.id);
        assert_eq!(l.groups()[0].selected, Some(a.id));
        l.close(a.id);
        assert_eq!(shape(&l.root), "0");
        assert_eq!(l.groups()[0].selected, None);
    }

    #[test]
    fn closing_another_groups_last_tab_removes_it_and_merges_the_axis() {
        let (a, b, c) = (code(), code(), code());
        let mut l = EditorLayout::single(vec![a, b, c]);
        let g = first_group(&l);
        let right = l.split(g, DropEdge::Right, b.id).unwrap();
        l.split(right, DropEdge::Bottom, c.id);
        assert_eq!(shape(&l.root), "H(1,V(1,1))");
        l.close(b.id);
        assert_eq!(shape(&l.root), "H(1,1)");
    }

    #[test]
    fn equalize_and_divider_fractions() {
        let (a, b, c) = (code(), code(), code());
        let mut l = EditorLayout::single(vec![a, b, c]);
        let g = first_group(&l);
        let r = l.split(g, DropEdge::Right, b.id).unwrap();
        l.split(r, DropEdge::Right, c.id);
        let LayoutNode::Split(s) = &l.root else {
            panic!("not a split")
        };
        let s = s.id;
        l.set_fractions(s, &[2.0, 1.0, 1.0]);
        assert_eq!(l.split_node(s).unwrap().fractions, [0.5, 0.25, 0.25]);
        l.set_fractions(s, &[1.0, 1.0]);
        assert_eq!(l.split_node(s).unwrap().fractions, [0.5, 0.25, 0.25]);
        l.equalize(s);
        let thirds: Vec<f64> = l
            .split_node(s)
            .unwrap()
            .fractions
            .iter()
            .map(|f| (f * 300.0).round())
            .collect();
        assert_eq!(thirds, [100.0, 100.0, 100.0]);
    }

    #[test]
    fn presets_keep_every_tab() {
        let items: Vec<EditorItem> = (0..4).map(|_| code()).collect();
        let mut l = EditorLayout::single(items.clone());
        l.apply(LayoutPreset::Three);
        assert_eq!(shape(&l.root), "H(4,V(0,0))");
        l.apply(LayoutPreset::TwoRows);
        assert_eq!(shape(&l.root), "V(4,0)");
        let g = first_group(&l);
        l.split(g, DropEdge::Right, items[3].id);
        l.apply(LayoutPreset::Single);
        let mut kept = ids(&l.items());
        let mut all = ids(&items);
        kept.sort();
        all.sort();
        assert_eq!(kept, all);
        assert_eq!(shape(&l.root), "4");
    }

    #[test]
    fn layouts_round_trip_through_json() {
        let mut voices = EditorItem::new(EditorContent::Audio(audio::Tab::Voices));
        voices.follows_selection = false;
        let mut l = EditorLayout::single(vec![
            code(),
            EditorItem::new(EditorContent::Graphics(gfx::Tab::Tilemap)),
            voices,
            EditorItem::new(EditorContent::Tutor),
        ]);
        let g = first_group(&l);
        let second = l.items()[1].id;
        l.split(g, DropEdge::Bottom, second);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<EditorLayout>(&json).unwrap(), l);
    }

    #[test]
    fn drop_zones() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 600.0,
        };
        let edge = DropZone::Edge;
        assert_eq!(drop_zone(150.0, 300.0, r), DropZone::Center);
        assert_eq!(drop_zone(10.0, 300.0, r), edge(DropEdge::Left));
        assert_eq!(drop_zone(290.0, 300.0, r), edge(DropEdge::Right));
        assert_eq!(drop_zone(150.0, 10.0, r), edge(DropEdge::Top));
        assert_eq!(drop_zone(150.0, 590.0, r), edge(DropEdge::Bottom));
        // The thresholds: just inside and just outside a third.
        assert_eq!(drop_zone(99.0, 300.0, r), edge(DropEdge::Left));
        assert_eq!(drop_zone(101.0, 300.0, r), DropZone::Center);
        assert_eq!(drop_zone(150.0, 199.0, r), edge(DropEdge::Top));
        // Corners: the nearer edge as a share of the side wins. 15% of the
        // width from the left loses to 5% of the height from the top.
        assert_eq!(drop_zone(45.0, 30.0, r), edge(DropEdge::Top));
        assert_eq!(drop_zone(15.0, 120.0, r), edge(DropEdge::Left));
        let none = Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        assert_eq!(drop_zone(0.0, 0.0, none), DropZone::Center);
    }

    /// A layout as the macOS shell writes it (`JSONEncoder`, sorted keys,
    /// pretty-printed) reads here, and writes back as the same JSON.
    #[test]
    fn a_layout_written_on_macos_reads_and_writes_back_the_same() {
        let swift = r#"{
  "focusedGroup" : "6A8B1C2D-0E4F-4A5B-8C7D-9E0F1A2B3C4D",
  "inspector" : true,
  "layout" : {
    "root" : {
      "split" : {
        "_0" : {
          "axis" : "horizontal",
          "children" : [
            {
              "group" : {
                "_0" : {
                  "id" : "6A8B1C2D-0E4F-4A5B-8C7D-9E0F1A2B3C4D",
                  "items" : [
                    {
                      "content" : { "code" : { "_0" : "assembly" } },
                      "followsSelection" : true,
                      "id" : "11111111-2222-4333-8444-555555555555"
                    },
                    {
                      "content" : { "audio" : { "_0" : "aram" } },
                      "followsSelection" : true,
                      "id" : "22222222-2222-4333-8444-555555555555"
                    }
                  ],
                  "selected" : "11111111-2222-4333-8444-555555555555"
                }
              }
            },
            {
              "group" : {
                "_0" : {
                  "id" : "7A8B1C2D-0E4F-4A5B-8C7D-9E0F1A2B3C4D",
                  "items" : [
                    {
                      "content" : { "tutor" : { } },
                      "followsSelection" : false,
                      "id" : "33333333-2222-4333-8444-555555555555"
                    },
                    {
                      "content" : { "graphics" : { "_0" : "tilemap" } },
                      "followsSelection" : true,
                      "id" : "44444444-2222-4333-8444-555555555555"
                    }
                  ],
                  "selected" : "33333333-2222-4333-8444-555555555555"
                }
              }
            }
          ],
          "fractions" : [ 0.6, 0.4 ],
          "id" : "8A8B1C2D-0E4F-4A5B-8C7D-9E0F1A2B3C4D"
        }
      }
    }
  },
  "sidebar" : false,
  "strip" : true,
  "tutorInDrawer" : false
}"#;
        let record: WorkspaceRecord = serde_json::from_str(swift).expect("reads");
        assert!(!record.sidebar && record.inspector);
        let items = record.layout.items();
        assert_eq!(
            items.iter().map(|i| i.content).collect::<Vec<_>>(),
            [
                EditorContent::Code(CodeRep::Assembly),
                EditorContent::Audio(audio::Tab::Aram),
                EditorContent::Tutor,
                EditorContent::Graphics(gfx::Tab::Tilemap),
            ]
        );
        assert_eq!(
            items[0].id.to_string(),
            "11111111-2222-4333-8444-555555555555"
        );
        let back = serde_json::to_value(&record).unwrap();
        let original: serde_json::Value = serde_json::from_str(swift).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn a_record_without_the_flags_reads_with_their_defaults() {
        let l = EditorLayout::single(vec![code()]);
        let json = format!(r#"{{"layout":{}}}"#, serde_json::to_string(&l).unwrap());
        let r: WorkspaceRecord = serde_json::from_str(&json).unwrap();
        assert!(r.sidebar && r.inspector && r.strip && !r.tutor_in_drawer);
        assert_eq!(r.focused_group, None);
        // Every flag is written, as the macOS shell needs them all.
        let out = serde_json::to_value(&r).unwrap();
        for key in ["sidebar", "inspector", "strip", "tutorInDrawer"] {
            assert!(out.get(key).is_some(), "{key} written");
        }
    }

    #[test]
    fn a_drop_round_trips_through_its_json() {
        let id = Id::new();
        for d in [
            TabDrop::Item(id),
            TabDrop::Open(EditorContent::Graphics(gfx::Tab::Oam)),
            TabDrop::OpenAt(CodeRep::Assembly, 0x80_8000),
        ] {
            assert_eq!(TabDrop::from_json(&d.to_json()), Some(d));
        }
        assert_eq!(TabDrop::from_json("not a drop"), None);
    }

    #[test]
    fn every_view_has_a_raw_name_that_reads_back() {
        for t in gfx::Tab::ALL {
            assert_eq!(gfx_from_raw(gfx_raw(t)), Some(t));
        }
        for t in audio::Tab::ALL {
            assert_eq!(audio_from_raw(audio_raw(t)), Some(t));
        }
        for r in CodeRep::ALL {
            assert_eq!(CodeRep::from_raw(r.raw()), Some(r));
        }
    }

    // MARK: Workspace

    #[test]
    fn a_new_workspace_is_one_group_holding_hex_with_focus() {
        let w = Workspace::default();
        assert_eq!(w.layout().groups().len(), 1);
        let f = w.focused_item().expect("a tab has focus");
        assert_eq!(f.content, EditorContent::Code(CodeRep::Hex));
        assert_eq!(w.current_code_item(), f.id);
    }

    #[test]
    fn a_second_tab_of_the_same_code_view_does_not_follow() {
        let mut w = Workspace::default();
        let hex = w.focused_item().unwrap().id;
        let twin = w.open(EditorContent::Code(CodeRep::Hex), None).unwrap();
        assert_ne!(twin, hex);
        assert!(!w.layout().item(twin).unwrap().follows_selection);
        let asm = w
            .open(EditorContent::Code(CodeRep::Assembly), None)
            .unwrap();
        assert!(w.layout().item(asm).unwrap().follows_selection);
        assert_eq!(w.focused_item().unwrap().id, asm);
    }

    #[test]
    fn closing_keeps_focus_on_a_group_that_exists_and_drops_the_tabs_models() {
        let mut w = Workspace::default();
        let hex = w.focused_item().unwrap().id;
        let asm = w
            .open(EditorContent::Code(CodeRep::Assembly), None)
            .unwrap();
        let g = w.focused_group();
        let right = w.split(g, DropEdge::Right, asm).unwrap();
        assert_eq!(w.focused_group(), right);
        w.decompiler(Some(asm));
        w.close(asm);
        assert_eq!(w.layout().groups().len(), 1);
        assert_eq!(w.focused_group(), g, "focus moves to a group still there");
        assert!(!w.decompilers.contains_key(&asm));
        assert_eq!(w.current_code_item(), hex);
    }

    #[test]
    fn restoring_keeps_the_saved_focus_or_takes_the_first_group() {
        let mut w = Workspace::default();
        let mut saved = EditorLayout::single(vec![code(), code()]);
        let g = first_group(&saved);
        let second = saved.items()[1].id;
        let right = saved.split(g, DropEdge::Right, second).unwrap();
        w.restore(saved.clone(), Some(right));
        assert_eq!(w.focused_group(), right);
        w.restore(saved, Some(Id::new()));
        assert_eq!(w.focused_group(), g);
    }
}
