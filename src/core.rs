//! `:core` / `:stg` / `:cmm` viewer. GHCi runs interpreted bytecode, so it
//! never sees optimized code. Instead we drop the expression into a temp
//! module, compile it with `-O2` and a dump flag, and highlight what comes out.

use std::collections::HashMap;
use std::process::Command;

use crate::error::{Error, Result};
use crate::ghc::LaunchMode;
use crate::{highlight, style};

#[derive(Clone, Copy)]
pub enum DumpKind {
    Core,
    Stg,
    Cmm,
}

impl DumpKind {
    fn dump_flag(self) -> &'static str {
        match self {
            DumpKind::Core => "-ddump-simpl",
            DumpKind::Stg => "-ddump-stg-final",
            DumpKind::Cmm => "-ddump-cmm",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DumpKind::Core => "core",
            DumpKind::Stg => "stg",
            DumpKind::Cmm => "cmm",
        }
    }
}

/// Name we bind the expression to. GHCi already calls the last result `it`,
/// so the dump looks familiar instead of showing some internal name.
const BINDING: &str = "it";

/// Compile `expr` under `-O2` and return the highlighted dump, or GHC's error
/// if it doesn't typecheck. `bindings` are the session's `let` definitions,
/// spliced in as top-level decls so `:core <name>` resolves prompt-defined names.
pub fn dump(
    kind: DumpKind,
    expr: &str,
    imports: &[String],
    bindings: &[String],
    mode: LaunchMode,
) -> Result<String> {
    let dir = std::env::temp_dir().join("ghcitty-core");
    std::fs::create_dir_all(&dir)?;
    let src = dir.join("GhcittyDump.hs");
    std::fs::write(&src, build_module(expr, imports, bindings))?;

    let out_dir = dir.to_string_lossy().to_string();
    let src_path = src.to_string_lossy().to_string();
    let flags = [
        "-O2",
        "-fforce-recomp",
        kind.dump_flag(),
        "-dsuppress-all",
        "-dsuppress-uniques",
        "-dno-typeable-binds",
        "-c",
        "-outputdir",
        &out_dir,
        &src_path,
    ];

    let output = run_ghc(mode, &flags)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let section = extract_section(&stdout);
    if section.trim().is_empty() {
        // No dump means it didn't compile; surface GHC's error verbatim.
        let msg = stderr.trim();
        let msg = if msg.is_empty() {
            "no output produced (does the expression typecheck?)".to_string()
        } else {
            msg.to_string()
        };
        return Err(Error::Ghc(msg));
    }
    let filtered = match kind {
        // Cmm comes as banner-separated chunks, not `name = ...` bindings.
        DumpKind::Cmm => filter_cmm(&section),
        DumpKind::Core | DumpKind::Stg => filter_reachable(&section),
    };
    Ok(render(kind, &strip_header(&filtered)))
}

fn build_module(expr: &str, imports: &[String], bindings: &[String]) -> String {
    let mut src = String::from("{-# OPTIONS_GHC -w #-}\nmodule GhcittyDump where\n");
    for imp in imports {
        src.push_str(imp);
        src.push('\n');
    }
    // Session `let` bindings become top-level decls. GHC treats module-level
    // bindings as mutually recursive, so order doesn't matter. Skip any binding
    // named `it`: it would clash with the wrapper we emit below.
    for binding in bindings {
        let binder: String = binding
            .trim_start()
            .chars()
            .take_while(|&c| is_ident_char(c))
            .collect();
        if binder == BINDING {
            continue;
        }
        src.push_str(binding);
        src.push('\n');
    }
    // Wrap in parens so a multi-token expression stays one expression, and
    // indent it, otherwise the layout rule sees a new top-level decl.
    src.push_str(&format!("{BINDING} =\n  (\n"));
    for line in expr.lines() {
        src.push_str("    ");
        src.push_str(line);
        src.push('\n');
    }
    src.push_str("  )\n");
    src
}

/// Run ghc the same way the REPL was launched (plain, stack or cabal) so the
/// temp module sees the same packages.
fn run_ghc(mode: LaunchMode, flags: &[&str]) -> Result<std::process::Output> {
    let mut cmd = match mode {
        LaunchMode::Plain => {
            let mut c = Command::new("ghc");
            c.args(flags);
            c
        }
        LaunchMode::Stack => {
            let mut c = Command::new("stack");
            c.arg("ghc").arg("--").args(flags);
            c
        }
        LaunchMode::Cabal => {
            let mut c = Command::new("cabal");
            c.args(["exec", "-v0", "--", "ghc"]).args(flags);
            c
        }
    };
    cmd.output()
        .map_err(|e| Error::Ghc(format!("failed to run compiler: {e}")))
}

/// Keep everything from the `==== ... ====` banner onward, dropping leading
/// noise and trailing blank lines.
fn extract_section(stdout: &str) -> String {
    let start = stdout
        .lines()
        .position(|l| l.trim_start().starts_with("===================="));
    let body = match start {
        Some(i) => stdout.lines().skip(i).collect::<Vec<_>>().join("\n"),
        None => return String::new(),
    };
    body.trim_end().to_string()
}

/// True for chars that can appear in a Core/STG identifier.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\'' || c == '#' || c == '$'
}

/// Every session `let` binding gets spliced into the temp module, so the dump
/// is full of stuff the user didn't ask about. Keep the banner and whatever
/// `it` actually uses. If the dump doesn't look the way we expect, return it
/// untouched.
fn filter_reachable(section: &str) -> String {
    // Put the banner and "Result size" lines aside, then split the rest into
    // bindings on blank lines.
    let mut header: Vec<&str> = Vec::new();
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in section.lines() {
        if is_header_line(line) {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
            header.push(line);
        } else if line.trim().is_empty() {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }

    // A block's binders are the names at column 0, ignoring `-- RHS size`
    // comments, `[IdInfo]` annotations and the `Rec {` / `end Rec }` markers.
    // A `Rec` group can define several names, so collect all of them.
    let binders = |block: &[&str]| -> Vec<String> {
        let mut names = Vec::new();
        for line in block {
            if line.starts_with(char::is_whitespace) {
                continue;
            }
            let t = line.trim_start();
            if t.starts_with("--")
                || t.starts_with('[')
                || t == "Rec {"
                || t.starts_with("end Rec")
                || t == "}"
            {
                continue;
            }
            let name: String = t.chars().take_while(|&c| is_ident_char(c)).collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
        names
    };

    let mut name_to_idx: HashMap<String, usize> = HashMap::new();
    for (i, block) in blocks.iter().enumerate() {
        for name in binders(block) {
            name_to_idx.insert(name, i);
        }
    }

    // Walk everything reachable from `it`. No `it` means the dump has a shape
    // we don't understand, so just show the whole thing.
    let Some(&root) = name_to_idx.get(BINDING) else {
        return section.to_string();
    };
    let mut keep = vec![false; blocks.len()];
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        if keep[i] {
            continue;
        }
        keep[i] = true;
        for line in &blocks[i] {
            for tok in line.split(|c: char| !is_ident_char(c)) {
                if let Some(&j) = name_to_idx.get(tok) {
                    if !keep[j] {
                        stack.push(j);
                    }
                }
            }
        }
    }

    let mut out: Vec<String> = header.iter().map(|l| l.to_string()).collect();
    for (i, block) in blocks.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        if !out.is_empty() {
            out.push(String::new());
        }
        out.extend(block.iter().map(|l| l.to_string()));
    }
    out.join("\n").trim_end().to_string()
}

/// GHC prints one `==== Output Cmm ====` banner per top-level symbol. Keep the
/// chunks that mention `it` (or floated-out `it1`, `it2`, ...) and drop the
/// ones that belong to spliced session bindings. If nothing matches, return
/// the whole section.
fn filter_cmm(section: &str) -> String {
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in section.lines() {
        if line
            .trim_start()
            .starts_with("==================== Output Cmm")
            && !cur.is_empty()
        {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push(line);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }

    let kept: Vec<String> = chunks
        .into_iter()
        .filter(|chunk| chunk.iter().any(|l| l.contains(BINDING)))
        .map(|chunk| chunk.join("\n").trim_end().to_string())
        .collect();
    if kept.is_empty() {
        return section.to_string();
    }
    kept.join("\n\n")
}

/// GHC's `==== ... ====` banners and the `Result size` line (plus its
/// `= {terms: ...}` continuation). Compiler log noise, not code.
fn is_header_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("====") || t.starts_with("Result size") || t.starts_with("= {")
}

/// Drop the header lines and squash the blank runs they leave behind, so
/// what's left is just the code.
fn strip_header(section: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in section.lines().filter(|l| !is_header_line(l)) {
        let blank = line.trim().is_empty();
        if blank && out.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim_end().to_string()
}

fn render(kind: DumpKind, section: &str) -> String {
    let mut out = style::dim()
        .paint(format!("-- {}, -O2", kind.label()))
        .to_string();
    out.push('\n');
    for line in section.lines() {
        out.push_str(&highlight::highlight_input(line));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_module_includes_imports_and_bindings() {
        let src = build_module(
            "y",
            &["import Data.List".to_string()],
            &["y = (\\x -> x + 1) 41".to_string()],
        );
        assert!(src.contains("module GhcittyDump where"));
        assert!(src.contains("import Data.List"));
        assert!(src.contains("y = (\\x -> x + 1) 41"));
        assert!(src.contains("it ="));
        assert!(src.contains("    y"));
    }

    #[test]
    fn extract_section_drops_leading_noise() {
        let stdout = "[1 of 1] Compiling GhcittyDump\n\
            ==================== Tidy Core ====================\n\
            Result size of Tidy Core = {terms: 1}\n\
            it = I# 1#\n\n\n";
        let section = extract_section(stdout);
        assert!(section.starts_with("===================="));
        assert!(section.contains("it = I# 1#"));
        assert!(!section.ends_with('\n'));
    }

    #[test]
    fn extract_section_empty_when_no_banner() {
        assert_eq!(extract_section("error: boom\n"), "");
    }

    #[test]
    fn filter_reachable_drops_unrelated_bindings() {
        let section = "==================== Final STG: ====================\n\
            y = IS! [42#];\n\n\
            it5 = I#! [2#];\n\n\
            it1 = :! [it5 []];\n\n\
            it = :! [it5 it1];";
        let kept = filter_reachable(section);
        assert!(kept.contains("it ="));
        assert!(kept.contains("it5 ="));
        assert!(kept.contains("it1 ="));
        // The stale `let y` binding isn't referenced, so it's gone.
        assert!(!kept.contains("y = IS!"));
        assert!(kept.starts_with("===================="));
    }

    #[test]
    fn filter_reachable_keeps_rec_worker_loop() {
        // `sum [..]` compiles to a recursive worker `$wgo3` inside a `Rec`
        // group. `it` calls it, so the whole loop has to survive the filter.
        let section = "==================== Tidy Core ====================\n\
            Result size of Tidy Core\n\
            \x20 = {terms: 27, types: 8, coercions: 0, joins: 0/0}\n\n\
            Rec {\n\
            $wgo3\n\
            \x20 = \\ x ww ->\n\
            \x20     case x of wild {\n\
            \x20       __DEFAULT -> $wgo3 (+# wild 1#) (+# ww (*# wild 2#));\n\
            \x20       100# -> +# ww 200#\n\
            \x20     }\n\
            end Rec }\n\n\
            it = case $wgo3 1# 0# of ww { __DEFAULT -> I# ww }";
        let kept = filter_reachable(section);
        assert!(kept.contains("Rec {"), "Rec block dropped: {kept}");
        assert!(kept.contains("$wgo3"));
        assert!(kept.contains("+# ww (*# wild 2#)"));
        assert!(kept.contains("it = case $wgo3"));
        // The `Result size` continuation stays attached to the header.
        assert!(kept.contains("= {terms: 27"));
    }

    #[test]
    fn filter_reachable_keeps_referenced_session_binding() {
        // `:core y` wraps `y` itself, so the wrapper references it.
        let section = "==================== Final STG: ====================\n\
            y = IS! [42#];\n\n\
            it = y;";
        let kept = filter_reachable(section);
        assert!(kept.contains("y = IS!"));
        assert!(kept.contains("it = y"));
    }

    #[test]
    fn filter_cmm_keeps_only_wrapper_sections() {
        let section = "==================== Output Cmm ====================\n\
            [section \"\"data\" . y_closure\" {\n\
            \x20    y_closure:\n\
            \x20        const I#_con_info;\n\
            \x20        const 42;\n\
            \x20}]\n\n\n\
            ==================== Output Cmm ====================\n\
            [section \"\"data\" . it_closure\" {\n\
            \x20    it_closure:\n\
            \x20        const I#_con_info;\n\
            \x20        const 2;\n\
            \x20}]";
        let kept = filter_cmm(section);
        assert!(kept.contains("it_closure"));
        assert!(!kept.contains("y_closure"));
    }

    #[test]
    fn filter_cmm_passes_through_when_nothing_matched() {
        let section = "==================== Output Cmm ====================\n\
            [section \"\"data\" . y_closure\" { y_closure: }]";
        assert_eq!(filter_cmm(section), section);
    }

    #[test]
    fn strip_header_leaves_just_the_code() {
        let section = "==================== Tidy Core ====================\n\
            Result size of Tidy Core\n\
            \x20 = {terms: 3, types: 1, coercions: 0, joins: 0/0}\n\n\
            it = I# 12#";
        assert_eq!(strip_header(section), "it = I# 12#");
    }

    #[test]
    fn strip_header_squashes_cmm_banner_gaps() {
        let section = "==================== Output Cmm ====================\n\
            [it_closure]\n\n\n\
            ==================== Output Cmm ====================\n\
            [it1_closure]";
        assert_eq!(strip_header(section), "[it_closure]\n\n[it1_closure]");
    }

    #[test]
    fn filter_reachable_passes_through_unknown_shape() {
        let section = "==================== Tidy Core ====================\nsomething weird";
        assert_eq!(filter_reachable(section), section);
    }
}
