//! The Bash commands a read-only Claude turn runs without asking. Claude's
//! `--allowedTools` patterns match any arguments, so `Bash(git diff *)` also
//! approved `git diff --ext-diff` and `Bash(rg *)` approved `rg --pre <cmd>`.
//! Read-only turns therefore allow no Bash pattern; every Bash call reaches
//! the permission prompt and only a command built from flags known to read
//! is approved here. Everything else is denied.

/// Long flags git accepts on `diff`, `log` and `show` that only change what
/// is printed. None is a prefix of an option that runs a program or writes a
/// file (`--output`, `--ext-diff`, `--textconv`), so git's abbreviation
/// matching cannot reach one.
const DIFF_LONG: &[&str] = &[
    "--stat",
    "--numstat",
    "--shortstat",
    "--dirstat",
    "--summary",
    "--name-only",
    "--name-status",
    "--patch",
    "--no-patch",
    "--raw",
    "--unified",
    "--word-diff",
    "--color",
    "--no-color",
    "--ignore-all-space",
    "--ignore-space-change",
    "--ignore-blank-lines",
    "--find-renames",
    "--no-renames",
    "--diff-filter",
    "--diff-algorithm",
    "--minimal",
    "--patience",
    "--histogram",
    "--abbrev",
    "--full-index",
    "--relative",
    "--check",
    "--no-ext-diff",
    "--no-textconv",
];
const DIFF_ONLY_LONG: &[&str] = &[
    "--cached",
    "--staged",
    "--merge-base",
    "--exit-code",
    "--quiet",
];
const LOG_LONG: &[&str] = &[
    "--oneline",
    "--format",
    "--pretty",
    "--graph",
    "--decorate",
    "--no-decorate",
    "--all",
    "--branches",
    "--tags",
    "--max-count",
    "--skip",
    "--since",
    "--after",
    "--until",
    "--before",
    "--author",
    "--committer",
    "--reverse",
    "--first-parent",
    "--merges",
    "--no-merges",
    "--follow",
    "--date",
    "--abbrev-commit",
    "--no-abbrev-commit",
    "--left-right",
    "--topo-order",
    "--date-order",
];
const STATUS_LONG: &[&str] = &[
    "--short",
    "--long",
    "--branch",
    "--porcelain",
    "--untracked-files",
    "--ignored",
    "--show-stash",
    "--verbose",
    "--renames",
    "--no-renames",
    "--ahead-behind",
    "--no-ahead-behind",
];

/// Whether `command` is a read-only listing or git query Claude may run.
pub(crate) fn allowed(command: &str) -> bool {
    // Anything the shell would interpret (chaining, pipes, redirection,
    // substitution, variables, quoting, globs, brace or newline) rules the
    // command out, so the words below are exactly what the program receives.
    if !command
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " -_./:=,@^~+%".contains(c))
    {
        return false;
    }
    let words: Vec<&str> = command.split(' ').filter(|w| !w.is_empty()).collect();
    match words.as_slice() {
        ["pwd"] => true,
        ["ls", args @ ..] => args.iter().all(|arg| match arg.strip_prefix('-') {
            Some(flags) => !flags.is_empty() && flags.chars().all(|c| c.is_ascii_alphanumeric()),
            None => true,
        }),
        ["git", "--no-pager", sub, args @ ..] | ["git", sub, args @ ..] => git(sub, args),
        _ => false,
    }
}

fn git(sub: &str, args: &[&str]) -> bool {
    let (long, short, counts): (&[&[&str]], &str, bool) = match sub {
        "status" => (&[STATUS_LONG], "sbuvz", false),
        "diff" => (&[DIFF_LONG, DIFF_ONLY_LONG], "puswbzRMCU", false),
        "log" | "show" => (&[DIFF_LONG, LOG_LONG], "puswbzRMCUn", true),
        _ => return false,
    };
    for (i, arg) in args.iter().enumerate() {
        if *arg == "--" {
            // Only paths follow.
            return args[i + 1..].iter().all(|path| !path.is_empty());
        }
        if let Some(name) = arg.strip_prefix("--") {
            let name = format!("--{}", name.split('=').next().unwrap_or(""));
            if !long.iter().any(|set| set.contains(&name.as_str())) {
                return false;
            }
        } else if let Some(flags) = arg.strip_prefix('-') {
            if !short_flags(flags, short, counts) {
                return false;
            }
        }
        // Anything else is a revision or a path.
    }
    true
}

/// A bundle of short flags such as `-sb`, `-U3`, `-n5` or `-20`.
fn short_flags(flags: &str, allowed: &str, counts: bool) -> bool {
    if flags.is_empty() {
        return false;
    }
    if counts && flags.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    for (i, c) in flags.char_indices() {
        if !allowed.contains(c) {
            return false;
        }
        if c == 'U' || c == 'n' {
            return flags[i + 1..].chars().all(|c| c.is_ascii_digit());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn everyday_read_only_commands_are_allowed() {
        for command in [
            "pwd",
            "ls",
            "ls -la src",
            "ls -1 crates/gespan-core",
            "git status",
            "git status -sb",
            "git status --porcelain=v2 --branch",
            "git diff",
            "git diff --cached --stat",
            "git diff HEAD~1 -- src/main.rs",
            "git diff -U10 main...HEAD",
            "git --no-pager diff --name-only",
            "git log --oneline -20",
            "git log -n5 --format=%h%x09%s",
            "git log -n 5 --since=2.weeks --author=vasanth",
            "git log --graph --decorate --all",
            "git show HEAD",
            "git show --stat HEAD^",
            "git show HEAD:crates/gespan-core/Cargo.toml",
        ] {
            assert!(allowed(command), "{command}");
        }
    }

    #[test]
    fn flags_that_run_programs_or_write_files_are_refused() {
        for command in [
            // ripgrep runs a preprocessor per file; the Grep tool covers search.
            "rg --pre ./evil pattern",
            "rg --pre-glob '*' --pre sh pattern",
            "rg pattern",
            // git external diff drivers, text conversion and output files.
            "git diff --ext-diff",
            "git diff --ext",
            "git diff --textconv HEAD",
            "git log -p --textconv",
            "git log --output=/tmp/owned",
            "git log --outp=/tmp/owned",
            "git show --output=notes.txt HEAD",
            "git diff -O/tmp/order",
            // git options before the subcommand override config and paths.
            "git -c core.pager=sh diff",
            "git -c diff.external=./evil diff",
            "git -C /etc status",
            "git --git-dir=/tmp/x log",
            "git --exec-path=/tmp diff",
            "git -p diff",
            // Subcommands other than the four queries.
            "git checkout main",
            "git stash",
            "git config core.pager sh",
            "git",
            "/usr/bin/git status",
        ] {
            assert!(!allowed(command), "{command}");
        }
    }

    #[test]
    fn shell_syntax_cannot_smuggle_a_second_command() {
        for command in [
            "git status && rm -rf .",
            "git status; touch owned",
            "git log | sh",
            "git diff > patch.diff",
            "git diff >> patch.diff",
            "ls < /dev/null",
            "ls $(touch owned)",
            "ls `touch owned`",
            "git log $GIT_FORMAT",
            "ls ${HOME}",
            "git status &",
            "git status\ntouch owned",
            "git status\ttouch",
            "GIT_EXTERNAL_DIFF=./evil git diff",
            "git log --format='%h %s'",
            "git log --format=\"%h\"",
            "git show HEAD\\ --output=x",
            // A glob could expand to a file named like a flag.
            "git diff *",
            "ls *",
            "git log -- src/{a,b}",
            "git log @{u}",
        ] {
            assert!(!allowed(command), "{command:?}");
        }
    }

    #[test]
    fn ls_and_pwd_take_only_short_flags() {
        assert!(!allowed("pwd -P"));
        assert!(!allowed("ls --hyperlink=always"));
        assert!(!allowed("ls -"));
        assert!(!allowed("lsof"));
    }

    #[test]
    fn short_flag_bundles_are_checked_flag_by_flag() {
        assert!(allowed("git diff -pw"));
        assert!(allowed("git log -20"));
        assert!(!allowed("git diff -20"), "a count is a log option");
        assert!(!allowed("git status -c"));
        assert!(!allowed("git diff -U3x"));
        assert!(!allowed("git log -nx"));
        assert!(!allowed("git diff -"));
    }

    #[test]
    fn everything_after_a_double_dash_is_a_path() {
        assert!(allowed("git log -- --output=x"));
        assert!(allowed("git diff HEAD -- -weird-file-name"));
    }
}
