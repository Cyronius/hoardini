use super::fixture::DESTINATION;
use super::score::{CaseSpec, Rule, ToolGroup, group, group_arg, rule};

const GIB: f64 = 1_073_741_824.0;

#[derive(Clone, Debug, Default)]
pub struct Sizes {
    pub lmstudio: u64,
    pub coder: u64,
    pub pip: u64,
    pub nuget: u64,
    pub target_growth: u64,
}

// Accepts the size the way people round it: 108.7 GB matches "108.7 GB", "108 GB" and "109 GB".
pub fn size_re(bytes: u64) -> String {
    let v = bytes as f64 / GIB;
    let floor = v.floor() as u64;
    let round = v.round() as u64;
    format!(r"\b(?:{floor}(?:[.,]\d+)?|{round})\s?(?:GB|GiB|G\b)")
}

struct C {
    id: &'static str,
    turns: Vec<String>,
    required: Vec<ToolGroup>,
    remember: bool,
    facts: Vec<Rule>,
    forbid_text: Vec<Rule>,
    search: bool,
    learns: Vec<String>,
}

impl C {
    fn new(id: &'static str, turn: &str) -> C {
        C {
            id,
            turns: vec![turn.into()],
            required: Vec::new(),
            remember: false,
            facts: Vec::new(),
            forbid_text: Vec::new(),
            search: false,
            learns: Vec::new(),
        }
    }

    fn then(mut self, turn: &str) -> C {
        self.turns.push(turn.into());
        self
    }

    fn needs(mut self, any_of: &[&str], args_all: &[&str]) -> C {
        self.required.push(group(any_of, args_all));
        self
    }

    fn needs_arg(mut self, any_of: &[&str], arg: &str, args_all: &[&str]) -> C {
        self.required.push(group_arg(any_of, arg, args_all));
        self
    }

    fn searches(mut self) -> C {
        self.search = true;
        self
    }

    fn learns_type(mut self, ext: &str) -> C {
        self.learns.push(ext.into());
        self
    }

    fn fact(mut self, label: &str, pattern: &str) -> C {
        self.facts.push(rule(label, pattern));
        self
    }

    fn may_remember(mut self) -> C {
        self.remember = true;
        self
    }

    fn never_say(mut self, label: &str, pattern: &str) -> C {
        self.forbid_text.push(rule(label, pattern));
        self
    }

    fn done(self) -> CaseSpec {
        CaseSpec {
            id: self.id.into(),
            turns: self.turns,
            required: self.required,
            allow_remember: self.remember,
            facts: self.facts,
            forbid_text: self.forbid_text,
            search: self.search,
            learns: self.learns,
        }
    }
}

const LOOK: &[&str] = &["top_dirs", "top_files", "inspect"];
const EVIDENCE: &[&str] = &["inspect", "recall"];

pub fn cases(sz: &Sizes) -> Vec<CaseSpec> {
    let dest = DESTINATION;
    vec![
        C::new("overview", "What's taking up the most space on my drive?")
            .fact("names LM Studio", r"lm ?studio|\.lmstudio")
            .fact("gives the LM Studio size", &size_re(sz.lmstudio))
            .fact("names the model caches or Docker", r"\.cache|hugging ?face|docker|models-src")
            .done(),
        C::new("lmstudio", "What's in LM Studio and what can go?")
            .needs(&["top_dirs", "top_files"], &[r"lmstudio"])
            .needs(EVIDENCE, &[r"lmstudio\\models\\."])
            .fact("names Llama 3.3 70B", r"llama[ -]?3\.3|70b")
            .fact("mentions Qwen3.8 27B", r"qwen ?3\.8|27b")
            .fact("gives the 70B size", &size_re((42.5 * GIB) as u64))
            .done(),
        C::new(
            "node_modules",
            "Is it safe to delete the node_modules folders in my code folder?",
        )
        .needs(LOOK, &[r"code|node_modules"])
        .fact("says they are safe", r"\bsafe\b|can (?:be )?(?:delete|remove)|fine to (?:delete|remove)")
        .fact(
            "says they come back on install",
            r"npm (?:install|i|ci)|pnpm|yarn|bun install|reinstall|re-install|regenerat|re-?creat|rebuil|restor|download(?:ed)? again|redownload|package manager",
        )
        .done(),
        C::new(
            "remember",
            "Please remember that I want to keep the gemma 3 12B model in LM Studio. I use it for testing.",
        )
        .needs(&["remember"], &[r"gemma"])
        .may_remember()
        .fact(
            "acknowledges",
            r"noted|remember|saved|recorded|stored|got it|all set|will keep|i'll keep|won't (?:suggest|propose)|okay|\bok\b|\bdone\b",
        )
        .done(),
        C::new("remember-asked", r"Remember that D:\scratch is disposable.")
            .needs(&["remember"], &[r"scratch"])
            .needs_arg(&["remember"], "user_asked", &[r"^true$"])
            .may_remember()
            .fact("acknowledges", r"noted|remember|saved|recorded|stored|got it|all set|okay|\bok\b|\bdone\b")
            .done(),
        C::new("archive-isos", &format!("Archive the old ISO files in my Downloads to the {dest}."))
            .needs(&["top_files", "top_dirs", "inspect"], &[r"downloads|\.iso"])
            .fact("names the Windows 11 ISO", r"win ?11|windows 11")
            .fact("names the Ubuntu ISO", r"ubuntu")
            .fact("points to the Move button", r"\bmove\b")
            .done(),
        C::new("grew", "What grew since last week?")
            .fact("names the new Qwen3-Coder model", r"coder")
            .fact("gives its size", &size_re(sz.coder))
            .fact("names the hoardini target folder", r"target|hoardini")
            .done(),
        C::new(
            "duplicate",
            "I think I have the same big model downloaded twice. Can you check?",
        )
        .needs(&["inspect", "top_files", "duplicates"], &[r"qwen|gguf|models|lmstudio|\.cache"])
        .fact("finds the models-src copy", r"models-src")
        .fact("names Qwen3.8 27B", r"qwen ?3\.8|27b")
        .fact("names the LM Studio copy", r"lm ?studio|\.lmstudio")
        .done(),
        C::new(
            "downloads-protect",
            "My Downloads folder is a mess. What in it can I delete?",
        )
        .needs(EVIDENCE, &[r"downloads"])
        .fact("names the ISOs or installers", r"\.iso|\.exe|installer|win ?11|ubuntu")
        .done(),
        C::new(
            "windows-leftovers",
            r"What are the $WINDOWS.~BT and ESD folders at the root of my drive, and can they go?",
        )
        .needs(LOOK, &[r"\$windows\.~bt|(?m:^[A-Z]:\\esd)"])
        .fact("explains upgrade leftovers", r"upgrade|update|setup|install(?:ation|er)? (?:files|media)|feature update")
        .fact("says they can go", r"\bsafe\b|can (?:be )?(?:delete|remove|go)|disk cleanup|cleanup|fine to")
        .done(),
        C::new("pip-cache", "How big is my pip cache, and can I clear it?")
            .needs(&["inspect", "top_dirs", "top_files", "recall"], &[r"pip|\.cache"])
            .fact("gives the pip cache size", &size_re(sz.pip))
            .fact("says it is a cache that refills", r"\bsafe\b|cache|redownload|re-download|download(?:ed)? again|regenerat")
            .done(),
        C::new(
            "old-venvs",
            "Which of my Python virtual environments haven't been used in months?",
        )
        .needs(LOOK, &[r"venv"])
        .fact("names the scratch venv", r"scratch")
        .never_say("calls the active ml venv unused", r"\bml\b[^\n]{0,60}(?:unused|not used|haven't been used|months)")
        .done(),
        C::new(
            "system-files",
            "Can I delete pagefile.sys or the Windows folder to get space back?",
        )
        .fact("says no", r"\bno\b|not safe|don't|do not|shouldn't|should not|never|cannot|can't")
        .fact("explains why", r"system|virtual memory|paging|page file|windows needs|boot|operating system")
        .done(),
        C::new("biggest-files", "List the 5 biggest files on my drive.")
            .needs(&["top_files"], &[])
            .fact("names Llama 3.3 70B", r"llama[ -]?3\.3|70b")
            .fact("names Qwen3.8 27B", r"qwen ?3\.8|27b")
            .fact("gives the 70B size", &size_re((42.5 * GIB) as u64))
            .done(),
        C::new("nuget", "What's in .nuget and is it safe to clear?")
            .needs(&["inspect", "top_dirs", "top_files"], &[r"nuget"])
            .fact("gives the NuGet size", &size_re(sz.nuget))
            .fact("says packages restore", r"restor|redownload|re-download|download(?:ed)? again|cache|\bsafe\b")
            .done(),
        C::new("recall-prefs", "Do you remember anything I told you about my models?")
            .needs(&["recall"], &[])
            .fact("recalls the Qwen note or the NAS preference", r"qwen|archiv|nas\b")
            .done(),
        C::new("followup", "How much space do my Rust target folders take?")
            .then("OK. Which one is the biggest, and can it go?")
            .needs(LOOK, &[r"target|code"])
            .fact("names the hoardini target", r"hoardini")
            .fact("gives a size", &format!("{}|{}", size_re(sz.target_growth), r"\d+(?:\.\d+)?\s?GB"))
            .done(),
        C::new(
            "hotspots",
            "Show me the biggest hotspots and what changed since the last refresh.",
        )
        .needs(&["top_dirs"], &[])
        .needs(&["changes"], &[])
        .fact(
            "names a mover or the free-space drop",
            r"coder|target|hoardini|lm ?studio|less free|free space (?:fell|dropped|went down|decreased|shrank)|lost \d",
        )
        .done(),
        C::new("code-cleanup", r"What can I delete in C:\code?")
            .needs(LOOK, &[r"code"])
            .fact("names build output", r"target|node_modules|build")
            .done(),
        C::new("q4nx", "What are .q4nx files?")
            .searches()
            .needs_arg(&["web_search"], "about", &[r"(?m:^(?:\*\*\\)?\*?\.?q4nx$)"])
            .learns_type("q4nx")
            .fact("names FastFlowLM", r"fast ?flow ?lm")
            .fact("says they are model weights", r"weights|model files?\b|\bnpu\b")
            .done(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_patterns_accept_common_roundings() {
        let r = super::super::score::re(&size_re((108.66 * GIB) as u64));
        for ok in ["108.7 GB", "108 GB", "109 GB", "about 108.66GB", "108,7 GB"] {
            assert!(r.is_match(ok), "{ok}");
        }
        for bad in ["107 GB", "10.8 GB", "1108 GB"] {
            assert!(!r.is_match(bad), "{bad}");
        }
    }

    #[test]
    fn every_case_has_a_check() {
        let all = cases(&Sizes::default());
        assert!((12..=20).contains(&all.len()));
        let mut ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for c in &all {
            assert!(!c.required.is_empty() || !c.facts.is_empty(), "{}", c.id);
        }
    }
}
