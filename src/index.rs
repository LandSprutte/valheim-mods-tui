//! Package index, filtering, and dependency-tree construction.

use crate::thunderstore::Mod;
use std::collections::{HashMap, HashSet};

/// Guard against pathological or malformed dependency chains.
const MAX_DEPTH: usize = 12;

/// Which slice of the catalogue the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Maintained mods carrying either 1.0 signal. The default.
    V1Ready,
    /// Only mods the author tagged for the Deep North / 1.0 game version.
    TaggedOnly,
    /// Only mods released on or after the 1.0 launch.
    UpdatedOnly,
    /// Everything, deprecated packages included.
    All,
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::V1Ready => "1.0 ready",
            Filter::TaggedOnly => "tagged only",
            Filter::UpdatedOnly => "updated only",
            Filter::All => "all mods",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Filter::V1Ready => "maintained, tagged for 1.0 or released since launch",
            Filter::TaggedOnly => "author-tagged 'Deep North Update'",
            Filter::UpdatedOnly => "released on or after 2026-09-09",
            Filter::All => "every Valheim package, deprecated included",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Filter::V1Ready => Filter::TaggedOnly,
            Filter::TaggedOnly => Filter::UpdatedOnly,
            Filter::UpdatedOnly => Filter::All,
            Filter::All => Filter::V1Ready,
        }
    }

    fn accepts(self, m: &Mod) -> bool {
        match self {
            Filter::V1Ready => m.supports_v1(),
            Filter::TaggedOnly => !m.deprecated && m.tagged_for_v1(),
            Filter::UpdatedOnly => !m.deprecated && m.updated_since_v1(),
            Filter::All => true,
        }
    }
}

/// How the root list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Downloads,
    Rating,
    Updated,
    Name,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Sort::Downloads => "downloads",
            Sort::Rating => "rating",
            Sort::Updated => "updated",
            Sort::Name => "name",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Sort::Downloads => Sort::Rating,
            Sort::Rating => Sort::Updated,
            Sort::Updated => Sort::Name,
            Sort::Name => Sort::Downloads,
        }
    }
}

pub struct Index {
    pub mods: Vec<Mod>,
    by_key: HashMap<String, usize>,
}

impl Index {
    pub fn new(mods: Vec<Mod>) -> Self {
        let by_key = mods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.full_name.clone(), i))
            .collect();
        Self { mods, by_key }
    }

    /// Strips the trailing version from a dependency string.
    ///
    /// Dependencies read "Owner-Name-1.2.3". Package names never contain a
    /// hyphen, so splitting once from the right always yields "Owner-Name"
    /// even for the handful of owners whose name contains a hyphen.
    pub fn dep_key(dep: &str) -> &str {
        dep.rsplit_once('-').map(|(k, _)| k).unwrap_or(dep)
    }

    pub fn dep_version(dep: &str) -> &str {
        dep.rsplit_once('-').map(|(_, v)| v).unwrap_or("")
    }

    pub fn lookup(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    pub fn get(&self, idx: usize) -> &Mod {
        &self.mods[idx]
    }

    /// Root entries for the list, filtered by game-version signal, free-text
    /// query, and sorted.
    pub fn roots(&self, filter: Filter, query: &str, sort: Sort) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<usize> = self
            .mods
            .iter()
            .enumerate()
            .filter(|(_, m)| filter.accepts(m))
            .filter(|(_, m)| {
                q.is_empty()
                    || m.name.to_lowercase().contains(&q)
                    || m.owner.to_lowercase().contains(&q)
                    || m.description.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();

        out.sort_by(|&a, &b| {
            let (x, y) = (&self.mods[a], &self.mods[b]);
            match sort {
                Sort::Downloads => y.downloads.cmp(&x.downloads),
                Sort::Rating => y.rating.cmp(&x.rating),
                Sort::Updated => y.date_updated.cmp(&x.date_updated),
                Sort::Name => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
            }
            .then_with(|| x.full_name.cmp(&y.full_name))
        });
        out
    }

    /// Direct dependencies of a mod, resolved against the index.
    pub fn deps(&self, idx: usize) -> Vec<Dep> {
        self.mods[idx]
            .dependencies
            .iter()
            .map(|d| {
                let key = Self::dep_key(d);
                match self.lookup(key) {
                    Some(i) => Dep::Known(i),
                    None => Dep::Missing(d.clone()),
                }
            })
            .collect()
    }

    /// Transitive install set for the chosen mods, dependencies first.
    ///
    /// Returns the install order plus any dependency that is not published on
    /// Thunderstore and therefore cannot be fetched automatically.
    pub fn install_plan(&self, chosen: &[usize]) -> (Vec<usize>, Vec<String>) {
        let mut order = Vec::new();
        let mut done = HashSet::new();
        let mut missing = Vec::new();
        let mut on_path = HashSet::new();

        for &root in chosen {
            self.visit_plan(root, &mut order, &mut done, &mut missing, &mut on_path, 0);
        }
        missing.sort();
        missing.dedup();
        (order, missing)
    }

    fn visit_plan(
        &self,
        idx: usize,
        order: &mut Vec<usize>,
        done: &mut HashSet<usize>,
        missing: &mut Vec<String>,
        on_path: &mut HashSet<usize>,
        depth: usize,
    ) {
        if done.contains(&idx) || depth > MAX_DEPTH {
            return;
        }
        // A cycle would otherwise recurse forever; the node still gets emitted
        // once the outer call for it unwinds.
        if !on_path.insert(idx) {
            return;
        }
        for dep in self.deps(idx) {
            match dep {
                Dep::Known(i) => self.visit_plan(i, order, done, missing, on_path, depth + 1),
                Dep::Missing(d) => missing.push(d),
            }
        }
        on_path.remove(&idx);
        if done.insert(idx) {
            order.push(idx);
        }
    }
}

pub enum Dep {
    Known(usize),
    Missing(String),
}

/// One rendered line of the mod list: a root mod or a nested dependency.
pub struct Row {
    /// Stable identity of this node within the tree, used for expand state.
    pub key: String,
    pub mod_idx: Option<usize>,
    /// Set when the dependency is not published on Thunderstore.
    pub missing: Option<String>,
    pub depth: usize,
    /// Pre-rendered box-drawing prefix.
    pub prefix: String,
    pub has_children: bool,
    pub expanded: bool,
    /// The dependency chain loops back on itself here.
    pub cycle: bool,
}

/// Flattens the root list into visible rows, expanding dependency subtrees.
pub fn build_rows(index: &Index, roots: &[usize], expanded: &HashSet<String>) -> Vec<Row> {
    let mut rows = Vec::with_capacity(roots.len());
    for &root in roots {
        let key = index.get(root).full_name.clone();
        push_node(
            index,
            Dep::Known(root),
            key,
            0,
            String::new(),
            true,
            expanded,
            &mut Vec::new(),
            &mut rows,
        );
    }
    rows
}

#[allow(clippy::too_many_arguments)]
fn push_node(
    index: &Index,
    node: Dep,
    key: String,
    depth: usize,
    prefix: String,
    is_last: bool,
    expanded: &HashSet<String>,
    ancestors: &mut Vec<usize>,
    rows: &mut Vec<Row>,
) {
    // Roots sit flush left; dependencies get a connector under their parent.
    let own_prefix = if depth == 0 {
        String::new()
    } else {
        format!("{}{}", prefix, if is_last { "└─ " } else { "├─ " })
    };

    let idx = match node {
        Dep::Missing(d) => {
            rows.push(Row {
                key,
                mod_idx: None,
                missing: Some(d),
                depth,
                prefix: own_prefix,
                has_children: false,
                expanded: false,
                cycle: false,
            });
            return;
        }
        Dep::Known(i) => i,
    };

    let cycle = ancestors.contains(&idx);
    let children = if cycle || depth >= MAX_DEPTH {
        Vec::new()
    } else {
        index.deps(idx)
    };
    let has_children = !children.is_empty();
    let is_expanded = has_children && expanded.contains(&key);

    rows.push(Row {
        key: key.clone(),
        mod_idx: Some(idx),
        missing: None,
        depth,
        prefix: own_prefix,
        has_children,
        expanded: is_expanded,
        cycle,
    });

    if !is_expanded {
        return;
    }

    // Continuation bar for deeper levels: a vertical rule unless this branch
    // was the last child, in which case the column goes blank.
    let child_prefix = if depth == 0 {
        String::new()
    } else {
        format!("{}{}", prefix, if is_last { "   " } else { "│  " })
    };

    ancestors.push(idx);
    let count = children.len();
    for (n, child) in children.into_iter().enumerate() {
        let child_key = match &child {
            Dep::Known(i) => format!("{}/{}", key, index.get(*i).full_name),
            Dep::Missing(d) => format!("{}/{}", key, d),
        };
        push_node(
            index,
            child,
            child_key,
            depth + 1,
            child_prefix.clone(),
            n + 1 == count,
            expanded,
            ancestors,
            rows,
        );
    }
    ancestors.pop();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thunderstore::Mod;

    fn m(full_name: &str, deps: &[&str]) -> Mod {
        let (owner, name) = full_name.split_once('-').unwrap();
        Mod {
            full_name: full_name.into(),
            name: name.into(),
            owner: owner.into(),
            description: String::new(),
            version: "1.0.0".into(),
            download_url: String::new(),
            package_url: String::new(),
            downloads: 0,
            rating: 0,
            file_size: 0,
            date_updated: "2026-09-10T00:00:00Z".into(),
            categories: vec![],
            dependencies: deps.iter().map(|d| format!("{d}-1.0.0")).collect(),
            deprecated: false,
        }
    }

    fn index() -> Index {
        Index::new(vec![
            m("Dev-App", &["Dev-Lib", "Dev-Ghost"]),
            m("Dev-Lib", &["Dev-Core"]),
            m("Dev-Core", &[]),
        ])
    }

    #[test]
    fn dependency_key_strips_the_version() {
        assert_eq!(Index::dep_key("Author-Mod-1.2.3"), "Author-Mod");
        assert_eq!(Index::dep_version("Author-Mod-1.2.3"), "1.2.3");
    }

    #[test]
    fn hyphenated_owner_names_still_resolve() {
        // A few Thunderstore owners contain a hyphen; package names never do,
        // so splitting once from the right stays correct.
        assert_eq!(Index::dep_key("Some-Team-CoolMod-0.1.0"), "Some-Team-CoolMod");
    }

    #[test]
    fn install_plan_lists_dependencies_before_dependents() {
        let idx = index();
        let app = idx.lookup("Dev-App").unwrap();
        let (plan, missing) = idx.install_plan(&[app]);
        let names: Vec<&str> = plan.iter().map(|&i| idx.get(i).full_name.as_str()).collect();

        assert_eq!(names, ["Dev-Core", "Dev-Lib", "Dev-App"]);
        assert_eq!(missing, ["Dev-Ghost-1.0.0"]);
    }

    #[test]
    fn shared_dependencies_are_installed_once() {
        let idx = index();
        let both = vec![idx.lookup("Dev-App").unwrap(), idx.lookup("Dev-Lib").unwrap()];
        let (plan, _) = idx.install_plan(&both);
        assert_eq!(plan.len(), 3, "Dev-Core must not be queued twice");
    }

    #[test]
    fn dependency_cycles_do_not_hang_the_planner() {
        let idx = Index::new(vec![m("Dev-A", &["Dev-B"]), m("Dev-B", &["Dev-A"])]);
        let (plan, _) = idx.install_plan(&[idx.lookup("Dev-A").unwrap()]);
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn collapsed_roots_render_one_row_each() {
        let idx = index();
        let roots = idx.roots(Filter::All, "", Sort::Name);
        let rows = build_rows(&idx, &roots, &HashSet::new());
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.depth == 0));
    }

    #[test]
    fn expanding_a_root_reveals_its_dependency_tree() {
        let idx = index();
        let roots = vec![idx.lookup("Dev-App").unwrap()];
        let mut open = HashSet::new();
        open.insert("Dev-App".to_string());
        open.insert("Dev-App/Dev-Lib".to_string());

        let rows = build_rows(&idx, &roots, &open);
        let shape: Vec<(usize, String)> = rows
            .iter()
            .map(|r| {
                let label = match r.mod_idx {
                    Some(i) => idx.get(i).full_name.clone(),
                    None => r.missing.clone().unwrap(),
                };
                (r.depth, label)
            })
            .collect();

        assert_eq!(
            shape,
            vec![
                (0, "Dev-App".into()),
                (1, "Dev-Lib".into()),
                (2, "Dev-Core".into()),
                (1, "Dev-Ghost-1.0.0".into()),
            ]
        );
        // The unpublished dependency is shown but cannot be selected.
        assert!(rows[3].mod_idx.is_none());
    }

    #[test]
    fn tree_prefixes_connect_branches() {
        let idx = index();
        let roots = vec![idx.lookup("Dev-App").unwrap()];
        let open: HashSet<String> = ["Dev-App", "Dev-App/Dev-Lib"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = build_rows(&idx, &roots, &open);

        assert_eq!(rows[0].prefix, "");
        assert_eq!(rows[1].prefix, "├─ ");
        // Dev-Core sits under a non-final branch, so the rule continues.
        assert_eq!(rows[2].prefix, "│  └─ ");
        assert_eq!(rows[3].prefix, "└─ ");
    }

    #[test]
    fn a_cyclic_branch_is_marked_and_not_re_expanded() {
        let idx = Index::new(vec![m("Dev-A", &["Dev-B"]), m("Dev-B", &["Dev-A"])]);
        let roots = vec![idx.lookup("Dev-A").unwrap()];
        let open: HashSet<String> = ["Dev-A", "Dev-A/Dev-B", "Dev-A/Dev-B/Dev-A"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = build_rows(&idx, &roots, &open);

        assert_eq!(rows.len(), 3);
        assert!(rows[2].cycle, "the repeated node must be flagged");
        assert!(!rows[2].has_children, "and must not expand further");
    }

    #[test]
    fn the_v1_filter_keeps_only_maintained_mods_with_a_signal() {
        let mut tagged = m("Dev-Tagged", &[]);
        tagged.date_updated = "2024-01-01T00:00:00Z".into();
        tagged.categories = vec![crate::thunderstore::V1_CATEGORY.into()];

        let mut old = m("Dev-Old", &[]);
        old.date_updated = "2024-01-01T00:00:00Z".into();

        let mut dead = m("Dev-Dead", &[]);
        dead.deprecated = true;

        // m() defaults to a post-launch date, so this one qualifies on recency.
        let fresh = m("Dev-Fresh", &[]);

        let idx = Index::new(vec![tagged, old, dead, fresh]);
        let kept: Vec<&str> = idx
            .roots(Filter::V1Ready, "", Sort::Name)
            .iter()
            .map(|&i| idx.get(i).full_name.as_str())
            .collect();

        assert_eq!(kept, ["Dev-Fresh", "Dev-Tagged"]);
        assert_eq!(idx.roots(Filter::All, "", Sort::Name).len(), 4);
    }

    #[test]
    fn search_matches_name_owner_and_description() {
        let mut m1 = m("Dev-Torches", &[]);
        m1.description = "keeps the night lit".into();
        let idx = Index::new(vec![m1, m("Other-Boats", &[])]);

        assert_eq!(idx.roots(Filter::All, "torch", Sort::Name).len(), 1);
        assert_eq!(idx.roots(Filter::All, "night", Sort::Name).len(), 1);
        assert_eq!(idx.roots(Filter::All, "dev", Sort::Name).len(), 1);
        assert_eq!(idx.roots(Filter::All, "nothing", Sort::Name).len(), 0);
    }
}
