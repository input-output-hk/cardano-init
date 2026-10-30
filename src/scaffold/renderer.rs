//! Render phase: produce the final bytes for every planned file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use minijinja::value::Value;
use minijinja::{AutoEscape, Environment, UndefinedBehavior};

use super::context::TemplateContext;
use super::planner::{FilePlan, TemplateSource};
use super::{ScaffoldError, TemplateAssets};

/// A file with its final content, ready to be written to disk.
#[derive(Debug)]
pub struct RenderedFile {
    /// Destination path relative to the project root.
    pub dest: PathBuf,
    pub content: Vec<u8>,
}

/// Render every entry of `plan` against `context`: renderable entries go through
/// `MiniJinja`, the rest are copied verbatim, and inline sources are used as-is.
///
/// # Errors
/// [`ScaffoldError::AssetNotFound`] for a missing template asset and
/// [`ScaffoldError::Render`] when a template fails to parse or render.
pub fn render(
    plan: &FilePlan,
    context: &TemplateContext,
) -> Result<Vec<RenderedFile>, ScaffoldError> {
    let sources = renderable_sources(plan)?;
    let env = environment(&sources)?;
    let ctx = Value::from_serialize(context);

    plan.entries
        .iter()
        .map(|entry| {
            let content = match &entry.source {
                TemplateSource::Inline(bytes) => bytes.clone(),
                source => {
                    let key = source
                        .asset_key()
                        .expect("only Inline sources lack an asset key");
                    if entry.render {
                        env.get_template(&key)
                            .and_then(|template| template.render(&ctx))
                            .map_err(|e| render_error(&key, e))?
                            .into_bytes()
                    } else {
                        TemplateAssets::require(&key)?.data.into_owned()
                    }
                }
            };
            Ok(RenderedFile {
                dest: entry.dest.clone(),
                content,
            })
        })
        .collect()
}

/// The text of every distinct template the plan renders, keyed by asset key.
fn renderable_sources(plan: &FilePlan) -> Result<BTreeMap<String, String>, ScaffoldError> {
    let mut sources = BTreeMap::new();
    let keys = plan
        .entries
        .iter()
        .filter(|entry| entry.render)
        .filter_map(|entry| entry.source.asset_key());
    for key in keys {
        if sources.contains_key(&key) {
            continue;
        }
        let asset = TemplateAssets::require(&key)?;
        // Embedded first-party asset: invalid UTF-8 is an authoring bug.
        let text = String::from_utf8(asset.data.into_owned())
            .expect("renderable template must be valid UTF-8");
        sources.insert(key, text);
    }
    Ok(sources)
}

/// A `MiniJinja` environment holding `sources`, configured per the rendering
/// contract (tech spec §4.3): undefined variables are errors, output is never
/// HTML-escaped, and a template's trailing newline is kept so output bytes
/// match the authored file (determinism, §11).
fn environment(sources: &BTreeMap<String, String>) -> Result<Environment<'_>, ScaffoldError> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_name| AutoEscape::None);
    env.set_keep_trailing_newline(true);
    for (key, text) in sources {
        env.add_template(key, text)
            .map_err(|e| render_error(key, e))?;
    }
    Ok(env)
}

fn render_error(path: &str, source: minijinja::Error) -> ScaffoldError {
    ScaffoldError::Render {
        path: path.to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::test_support::{a, registry};
    use super::*;
    use crate::registry::types::{Network, Role, RoleAssignment, Selection};
    use crate::scaffold::context::build_context;
    use crate::scaffold::planner;

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "test-project".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

    #[test]
    fn static_file_passes_through() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let validator = files
            .iter()
            .find(|f| f.dest.to_str().unwrap().contains("validators/giftcard.ak"))
            .expect("giftcard.ak should be in rendered files");

        let content = std::str::from_utf8(&validator.content).unwrap();
        assert!(content.contains("validator gift_card"));
        assert!(!content.contains("{{"));
    }

    #[test]
    fn jinja_template_renders_context() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let justfile = files
            .iter()
            .find(|f| f.dest == Path::new("Justfile"))
            .expect("Justfile should be in rendered files");

        let content = std::str::from_utf8(&justfile.content).unwrap();
        assert!(content.contains("test-project"));
        assert!(content.contains("build-on-chain"));
        assert!(!content.contains("{%"));
    }

    #[test]
    fn inline_source_produces_empty_content() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let gitkeep = files
            .iter()
            .find(|f| f.dest == Path::new("blueprint/.gitkeep"))
            .expect("blueprint/.gitkeep should be in rendered files");

        assert!(gitkeep.content.is_empty());
    }

    #[test]
    fn nix_flake_renders_with_packages() {
        let mut sel = selection(vec![a(Role::OnChain, "aiken")]);
        sel.nix = true;
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let flake = files
            .iter()
            .find(|f| f.dest == Path::new("flake.nix"))
            .expect("flake.nix should be in rendered files");

        let content = std::str::from_utf8(&flake.content).unwrap();
        assert!(content.contains("aiken"));
        assert!(content.contains("test-project"));
        assert!(!content.contains("{%"));
    }

    // -----------------------------------------------------------------------
    // Snapshot tests (determinism guard, §11)
    //
    // Render a selection into one canonical string (each file in plan order,
    // prefixed by its dest) and compare against a committed fixture under
    // `src/scaffold/snapshots/`. Regenerate after an intentional change with:
    //
    //     UPDATE_SNAPSHOTS=1 cargo test
    //
    // and review the diff before committing.
    // -----------------------------------------------------------------------

    fn render_to_snapshot(sel: &Selection) -> String {
        let reg = registry();
        let plan = planner::plan(sel, &reg).unwrap();
        let ctx = build_context(sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let mut out = String::new();
        for file in &files {
            out.push_str("=== ");
            out.push_str(&file.dest.to_string_lossy());
            out.push_str(" ===\n");
            out.push_str(std::str::from_utf8(&file.content).expect("snapshot content is UTF-8"));
            out.push('\n');
        }
        out
    }

    fn assert_snapshot(name: &str, sel: &Selection) {
        let actual = render_to_snapshot(sel);
        let path = format!(
            "{}/src/scaffold/snapshots/{name}.snap",
            env!("CARGO_MANIFEST_DIR")
        );
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("missing snapshot '{path}'. Run `UPDATE_SNAPSHOTS=1 cargo test` to create it.")
        });
        assert_eq!(actual, expected, "snapshot '{name}' drifted");
    }

    fn sel(assignments: Vec<RoleAssignment>, network: Network, nix: bool) -> Selection {
        Selection {
            project_name: "snap-project".to_string(),
            assignments,
            network,
            nix,
            spec: true,
        }
    }

    #[test]
    fn snapshot_aiken_only() {
        assert_snapshot(
            "aiken_only",
            &sel(vec![a(Role::OnChain, "aiken")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_plinth_nix() {
        // Plinth on-chain with `--nix`: locks the component-local haskell.nix
        // flake (mirroring IntersectMBO/plinth-template) and confirms Plinth's
        // packages stay out of the aggregated top-level shell.
        assert_snapshot(
            "plinth_nix",
            &sel(vec![a(Role::OnChain, "plinth")], Network::Preview, true),
        );
    }

    #[test]
    fn snapshot_plinth_meshjs_nix() {
        // Plinth on-chain + MeshJS off-chain under `--nix`: the top-level flake
        // composes the on-chain haskell.nix shell via `inputsFrom` *and* still
        // lists MeshJS's plain nixpkgs package (nodejs) — the two coexist.
        assert_snapshot(
            "plinth_meshjs_nix",
            &sel(
                vec![a(Role::OnChain, "plinth"), a(Role::OffChain, "meshjs")],
                Network::Preview,
                true,
            ),
        );
    }

    #[test]
    fn snapshot_aiken_meshjs_nix() {
        assert_snapshot(
            "aiken_meshjs_nix",
            &sel(
                vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
                Network::Preprod,
                true,
            ),
        );
    }

    #[test]
    fn snapshot_offchain_only() {
        assert_snapshot(
            "offchain_only",
            &sel(vec![a(Role::OffChain, "meshjs")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_yaci_devnet_only() {
        assert_snapshot(
            "yaci_devnet_only",
            &sel(vec![a(Role::Devnet, "yaci")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_aiken_meshjs_yaci() {
        // The headline integration: on-chain + off-chain + a local devnet in the
        // devnet role. Locks the connection wiring (MeshJS ↔ Yaci via .env).
        assert_snapshot(
            "aiken_meshjs_yaci",
            &sel(
                vec![
                    a(Role::OnChain, "aiken"),
                    a(Role::OffChain, "meshjs"),
                    a(Role::Devnet, "yaci"),
                ],
                Network::Preview,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_infra_kupo_ogmios() {
        // Aggregated infrastructure: two providers collapse into one infra/
        // component (the cardano-up driver). Infra-only ⇒ no blueprint dir.
        assert_snapshot(
            "infra_kupo_ogmios",
            &sel(
                vec![
                    a(Role::Infrastructure, "kupo"),
                    a(Role::Infrastructure, "ogmios"),
                ],
                Network::Preprod,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_infra_dolos() {
        // Dolos provides its own node socket (no cardano-node dependency), so it
        // OVERRIDES the base NODE_SOCKET_PATH default (DOLOS_SOCKET_PATH) and adds
        // DOLOS_GRPC_URL — locks the §5.4 explicit-over-default resolution.
        assert_snapshot(
            "infra_dolos",
            &sel(
                vec![a(Role::Infrastructure, "dolos")],
                Network::Preprod,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_aiken_meshjs_kupo_ogmios() {
        // Full stack: on-chain + off-chain + aggregated infra. Locks the .env
        // wiring (MeshJS reads INDEXER_URL/OGMIOS_URL written by the infra driver).
        assert_snapshot(
            "aiken_meshjs_kupo_ogmios",
            &sel(
                vec![
                    a(Role::OnChain, "aiken"),
                    a(Role::OffChain, "meshjs"),
                    a(Role::Infrastructure, "kupo"),
                    a(Role::Infrastructure, "ogmios"),
                ],
                Network::Preprod,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_scalus_fullstack() {
        // Scalus fills both on-chain and off-chain and declares a [fullstack]
        // template, so the two collapse into a single `protocol/` component
        // (no separate on-chain/ or off-chain/). Locks the fullstack collapse.
        assert_snapshot(
            "scalus_fullstack",
            &sel(
                vec![a(Role::OnChain, "scalus"), a(Role::OffChain, "scalus")],
                Network::Preview,
                false,
            ),
        );
    }

    #[test]
    fn rendered_output_has_no_crlf() {
        let s = sel(
            vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
            Network::Preview,
            true,
        );
        let reg = registry();
        let plan = planner::plan(&s, &reg).unwrap();
        let ctx = build_context(&s, &reg).unwrap();
        for file in render(&plan, &ctx).unwrap() {
            assert!(
                !file.content.contains(&b'\r'),
                "{:?} contains a CR byte",
                file.dest
            );
        }
    }

    #[test]
    fn rendered_justfile_keeps_trailing_newline() {
        let s = sel(vec![a(Role::OnChain, "aiken")], Network::Preview, false);
        let reg = registry();
        let plan = planner::plan(&s, &reg).unwrap();
        let ctx = build_context(&s, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();
        let justfile = files
            .iter()
            .find(|f| f.dest == Path::new("Justfile"))
            .unwrap();
        assert!(justfile.content.ends_with(b"\n"));
    }

    #[test]
    fn full_plan_renders_without_error() {
        let sel = selection(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        assert_eq!(files.len(), plan.entries.len());

        for file in &files {
            if file.dest != Path::new("blueprint/.gitkeep") {
                assert!(
                    !file.content.is_empty(),
                    "file {:?} should have content",
                    file.dest
                );
            }
        }
    }
}
