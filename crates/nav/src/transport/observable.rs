use super::*;

/// A green quest-list row is an observable proof of a script varp threshold
/// only when exactly one journal-colour call owns that row and its complete
/// value is at least the transport's required value. A shared row (Shield of
/// Arrav's two gang progress varps, for example) proves neither varp.
pub(super) struct JournalLinks {
    by_varp: HashMap<String, (String, i32)>,
    constants: HashMap<String, i32>,
}

impl JournalLinks {
    pub(super) fn from_content(content_root: &Path) -> Self {
        let constants = script_constants(content_root);
        let journal = fs::read_to_string(content_root.join("scripts/general/scripts/quests.rs2"))
            .unwrap_or_default();
        let interface =
            fs::read_to_string(content_root.join("scripts/player/interfaces/questlist.if"))
                .unwrap_or_default();
        // A green row must mean progress >= the call's completion argument.
        // A renamed or different colour helper supplies no route proof.
        let colour_rule = journal
            .split_once("[proc,send_quest_progress_colour]")
            .and_then(|(_, body)| body.split("\n[").next());
        if !colour_rule.is_some_and(|body| {
            body.contains("} else if ($progress >= $complete_progress) {")
                && body.contains("if_setcolour($component, ^green_rgb);")
        }) {
            return Self {
                by_varp: HashMap::new(),
                constants,
            };
        }
        let mut names = HashMap::new();
        let mut row = None;
        for line in interface.lines().map(str::trim) {
            if let Some(name) = config_header(line) {
                row = Some(name);
            } else if let (Some(row), Some(name)) = (row, line.strip_prefix("text=")) {
                if !name.trim().is_empty() {
                    names.insert(row.to_string(), name.trim().to_string());
                }
            }
        }
        let mut links = Vec::new();
        let mut row_uses = HashMap::<String, usize>::new();
        let mut varp_uses = HashMap::<String, usize>::new();
        for line in journal.lines() {
            let Some(args) = line
                .trim()
                .strip_prefix("~send_quest_progress_colour(")
                .and_then(|line| line.strip_suffix(");"))
            else {
                continue;
            };
            let mut args = args.split(',').map(str::trim);
            let Some(row) = args.next().and_then(|arg| arg.strip_prefix("questlist:")) else {
                continue;
            };
            *row_uses.entry(row.to_string()).or_default() += 1;
            let (Some(varp), Some(complete), None) = (
                args.next().and_then(|arg| arg.strip_prefix('%')),
                args.next().and_then(|arg| arg.strip_prefix('^')),
                args.next(),
            ) else {
                continue;
            };
            *varp_uses.entry(varp.to_string()).or_default() += 1;
            links.push((row, varp, complete));
        }
        let mut by_varp = HashMap::new();
        for (row, varp, complete) in links {
            if row_uses.get(row) != Some(&1) || varp_uses.get(varp) != Some(&1) {
                continue;
            }
            if let (Some(name), Some(&complete)) = (names.get(row), constants.get(complete)) {
                by_varp.insert(varp.to_string(), (name.clone(), complete));
            }
        }
        Self { by_varp, constants }
    }

    pub(super) fn constant(&self, name: &str) -> Option<i32> {
        self.constants.get(name).copied()
    }

    pub(super) fn completed_name(&self, varp: &str, min: i32) -> Option<&str> {
        let (name, complete) = self.by_varp.get(varp)?;
        (*complete >= min).then_some(name.as_str())
    }
}

/// Read `scripts/**/*.varp` declarations, excluding the historical
/// `_unpack` snapshots. Absence of a definition or `transmit=yes` means the
/// server does not send that varp to the live client.
fn transmitted_varps(content_root: &Path) -> HashSet<i32> {
    let ids = varp_ids_by_name(content_root);
    let mut definitions = HashMap::<String, bool>::new();
    let mut pending = vec![content_root.join("scripts")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "_unpack") {
                    pending.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|extension| extension == "varp")
            {
                let Ok(text) = fs::read_to_string(path) else {
                    continue;
                };
                let mut current = None;
                let mut transmitted = false;
                for line in text.lines().map(str::trim) {
                    if let Some(name) = config_header(line) {
                        if let Some(prev) = current.replace(name) {
                            definitions
                                .entry(prev.to_string())
                                .and_modify(|value| *value &= transmitted)
                                .or_insert(transmitted);
                        }
                        transmitted = false;
                    } else if line == "transmit=yes" {
                        transmitted = true;
                    }
                }
                if let Some(prev) = current {
                    definitions
                        .entry(prev.to_string())
                        .and_modify(|value| *value &= transmitted)
                        .or_insert(transmitted);
                }
            }
        }
    }
    definitions
        .into_iter()
        .filter_map(|(name, yes)| yes.then(|| ids.get(&name).copied()).flatten())
        .collect()
}

#[derive(Debug, Default)]
pub(crate) struct VarpGateAudit {
    pub(crate) converted: usize,
    pub(crate) omitted: HashMap<i32, usize>,
}

/// Shared source facts for transport producers. Resolve each gated edge
/// before it enters the graph: a non-transmitted varp must have a unique
/// completed-journal implication, otherwise the producer omits the edge.
pub(super) struct ObservableGates {
    transmitted: HashSet<i32>,
    names_by_id: HashMap<i32, String>,
    journal: JournalLinks,
}

impl ObservableGates {
    pub(super) fn from_content(content_root: &Path) -> Self {
        Self {
            transmitted: transmitted_varps(content_root),
            names_by_id: varp_ids_by_name(content_root)
                .into_iter()
                .map(|(name, id)| (id, name))
                .collect(),
            journal: JournalLinks::from_content(content_root),
        }
    }

    pub(super) fn admit_edge(
        &self,
        graph: &mut TransportGraph,
        mut edge: TransportEdge,
        audit: &mut VarpGateAudit,
    ) {
        if edge.varp_req.is_empty() {
            graph.edges.push(edge);
            return;
        }
        let mut proofs = Vec::new();
        for &(id, min) in &edge.varp_req {
            if self.transmitted.contains(&id) {
                continue;
            }
            let name = self
                .names_by_id
                .get(&id)
                .and_then(|varp| self.journal.completed_name(varp, min));
            let Some(name) = name else {
                *audit.omitted.entry(id).or_default() += 1;
                return;
            };
            proofs.push(name);
        }
        audit.converted += proofs.len();
        edge.quest_req.extend(proofs.into_iter().map(str::to_owned));
        edge.varp_req
            .retain(|(id, _)| self.transmitted.contains(id));
        graph.edges.push(edge);
    }
}

/// Independent pack-boundary assertion on the graph the producers emitted.
/// A new producer that bypasses [`ObservableGates::admit_edge`] cannot put
/// an untransmitted varp into a release pack unnoticed.
pub(crate) fn assert_transmitted_varp_reqs(content_root: &Path, graph: &TransportGraph) {
    let transmitted = transmitted_varps(content_root);
    for edge in graph.edges.iter().chain(&graph.teleports) {
        for (id, _) in &edge.varp_req {
            assert!(
                transmitted.contains(id),
                "producer emitted non-transmitted varp {id} on {:?} loc {}",
                edge.kind,
                edge.loc_id
            );
        }
    }
}
