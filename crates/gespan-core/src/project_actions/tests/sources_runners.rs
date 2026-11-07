use super::super::sources::runners::{
    package_manager, CargoAlias, Deno, Just, Makefile, Mise, PackageJson, Procfile,
};
use super::super::sources::Source;
use super::super::tasks_json::resolve;
use super::super::{Ctx, Found};
use std::path::Path;

const NEEDS_ARGUMENTS: &str = "Needs arguments, which Gespan can't pass yet";

fn parse_with(manager: &'static str, source: &dyn Source, file: &str, text: &str) -> Vec<Found> {
    let ctx = Ctx {
        checkout: Path::new("/work/app"),
        package_manager: manager,
    };
    assert!(
        source.files().contains(&file),
        "{file} isn't a {} file",
        source.id()
    );
    source.parse(&ctx, file, text).unwrap()
}

fn parse(source: &dyn Source, file: &str, text: &str) -> Vec<Found> {
    parse_with("npm", source, file, text)
}

/// The command line an action would run, as the resolver writes it.
fn line(source: &'static str, file: &str, found: &Found) -> String {
    let ctx = Ctx {
        checkout: Path::new("/work/app"),
        package_manager: "npm",
    };
    resolve(&ctx, source, file, found.clone()).command
}

fn labels(found: &[Found]) -> Vec<&str> {
    found.iter().map(|found| found.label.as_str()).collect()
}

#[test]
fn source_ids_and_files_are_stable() {
    let all: [(&dyn Source, &str, &[&str]); 7] = [
        (&PackageJson, "package.json", &["package.json"]),
        (&Deno, "deno", &["deno.json", "deno.jsonc"]),
        (&Mise, "mise", &["mise.toml", ".mise.toml"]),
        (&Just, "just", &["justfile", "Justfile", ".justfile"]),
        (&CargoAlias, "cargo", &[".cargo/config.toml"]),
        (&Procfile, "procfile", &["Procfile"]),
        (&Makefile, "make", &["Makefile", "makefile", "GNUmakefile"]),
    ];
    for (source, id, files) in all {
        assert_eq!(source.id(), id);
        assert_eq!(source.files(), files);
    }
}

#[test]
fn package_manager_follows_the_lockfile() {
    let cases = [
        (&["bun.lock"][..], "bun"),
        (&["bun.lockb"], "bun"),
        (&["pnpm-lock.yaml"], "pnpm"),
        (&["yarn.lock"], "yarn"),
        (&["package-lock.json"], "npm"),
        (&[], "npm"),
        // The first one in the table wins when several are present.
        (&["yarn.lock", "pnpm-lock.yaml"], "pnpm"),
        (&["yarn.lock", "bun.lock", "pnpm-lock.yaml"], "bun"),
    ];
    for (files, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            std::fs::write(dir.path().join(file), "").unwrap();
        }
        assert_eq!(package_manager(dir.path()), expected, "{files:?}");
    }
}

#[test]
fn package_scripts_skip_pre_and_post_hooks() {
    let text = r#"{ "config": { "scripts": { "zz": "not a script" } }, "scripts": {
        "build": "tsc",
        "prebuild": "rm -rf dist",
        "postbuild": "echo done",
        "predeploy": "echo orphan hook, no base script",
        "pre": "echo a script that is only named pre",
        "test": "vitest",
        "notascript": 5
    } }"#;
    let found = parse_with("pnpm", &PackageJson, "package.json", text);
    // In the order the file lists them; a nested `scripts` doesn't count.
    assert_eq!(labels(&found), ["build", "predeploy", "pre", "test"]);
    let build = &found[0];
    assert_eq!(build.detail.as_deref(), Some("tsc"));
    assert!(!build.background);
    assert_eq!(
        line("package.json", "package.json", build),
        "pnpm run build"
    );
}

#[test]
fn package_script_names_cannot_change_the_command() {
    let text = r#"{ "scripts": { "a b": "x", "dev;rm -rf /": "y" } }"#;
    let found = parse(&PackageJson, "package.json", text);
    let lines: Vec<_> = found
        .iter()
        .map(|found| line("package.json", "package.json", found))
        .collect();
    if cfg!(windows) {
        assert_eq!(lines, [r#"npm run "a b""#, r#"npm run "dev;rm -rf /""#]);
    } else {
        assert_eq!(lines, ["npm run 'a b'", "npm run 'dev;rm -rf /'"]);
    }
}

#[test]
fn package_json_without_scripts_lists_nothing() {
    assert!(parse(&PackageJson, "package.json", r#"{ "name": "x" }"#).is_empty());
    assert!(PackageJson
        .parse(
            &Ctx {
                checkout: Path::new("/w"),
                package_manager: "npm"
            },
            "package.json",
            "[1]"
        )
        .is_err());
}

#[test]
fn deno_tasks_string_and_object() {
    let text = r#"{
        // jsonc is fine
        "tasks": {
            "dev": "deno run -A main.ts",
            "check": { "command": "deno check .", "description": "Type-check" },
            "all": { "dependencies": ["dev", "check"] }
        }
    }"#;
    let found = parse(&Deno, "deno.jsonc", text);
    assert_eq!(labels(&found), ["dev", "check", "all"]);
    assert_eq!(found[0].detail.as_deref(), Some("deno run -A main.ts"));
    assert_eq!(found[1].detail.as_deref(), Some("Type-check"));
    assert_eq!(found[2].detail, None);
    assert_eq!(line("deno", "deno.json", &found[1]), "deno task check");
}

#[test]
fn mise_tasks() {
    let text = r#"
[tools]
node = "22"

[tasks.build]
description = "Build it"
run = "cargo build"

[tasks."test:unit"]
run = "cargo test"

[tasks.lint]
run = "cargo clippy"
hide = true

[tasks]
fmt = "cargo fmt"
"#;
    let found = parse(&Mise, ".mise.toml", text);
    // In file order, the `[tasks]` table's own keys after the sub-tables.
    assert_eq!(labels(&found), ["build", "test:unit", "lint", "fmt"]);
    assert_eq!(found[0].detail.as_deref(), Some("Build it"));
    assert_eq!(found[1].detail, None);
    assert!(found[2].hide);
    assert!(!found[0].hide);
    assert_eq!(line("mise", "mise.toml", &found[1]), "mise run test:unit");
}

#[test]
fn just_recipes_skip_private_and_take_doc_comments() {
    let text = "\
set shell := [\"bash\", \"-c\"]
version := \"1\"
export FLAG := \"x\"
alias b := build

# Build everything
build:
    cargo build

# Run the tests
test target='all': build
    cargo test

@quiet:
    echo hi

_helper:
    echo hidden

[private]
secret:
    echo hidden

# Documented but private
[private]
other:
    echo hidden

deploy env:
    ./deploy {{env}}

[no-cd]
tidy:
    echo ok
";
    let found = parse(&Just, "justfile", text);
    assert_eq!(labels(&found), ["build", "test", "quiet", "deploy", "tidy"]);
    assert_eq!(found[0].detail.as_deref(), Some("Build everything"));
    assert_eq!(found[1].detail.as_deref(), Some("Run the tests"));
    assert_eq!(found[2].detail, None);
    assert_eq!(line("just", "justfile", &found[0]), "just build");
    // `test` has a default for its only parameter, so it runs as is.
    assert_eq!(found[1].blocked, None);
    assert_eq!(line("just", "justfile", &found[1]), "just test");
    assert_eq!(found[3].blocked.as_deref(), Some(NEEDS_ARGUMENTS));
}

#[test]
fn just_recipes_run_when_every_parameter_has_a_default() {
    let text = "\
plain:
quoted a='1' b=\"two words\":
variadic mode=\"debug\" *flags: plain
colon url='http://x:1' :
expression x=(a + \"b:c\") y=VAR:
exported $port='80':
plus_default +flags='-q':
triple x='''it's here''' y=\"\"\"a \"b\" c\"\"\":
call target=arch():
call_args x=env_var_or_default(\"A\", \"b:)\") y=join(arch(), 'x'):
required target:
required_plus +files:
required_exported $env:
mixed a='1' b:
alias p := plain
x := 'not a recipe'
set dotenv-load := true
";
    let found = parse(&Just, "justfile", text);
    let runnable = [
        "plain",
        "quoted",
        "variadic",
        "colon",
        "expression",
        "exported",
        "plus_default",
        "triple",
        "call",
        "call_args",
    ];
    let blocked = ["required", "required_plus", "required_exported", "mixed"];
    let expected: Vec<_> = runnable.iter().chain(&blocked).copied().collect();
    assert_eq!(labels(&found), expected);
    for found in &found {
        let name = found.label.as_str();
        assert_eq!(line("just", "justfile", found), format!("just {name}"));
        let reason = blocked.contains(&name).then_some(NEEDS_ARGUMENTS);
        assert_eq!(found.blocked.as_deref(), reason, "{name}");
    }
}

#[test]
fn just_lists_a_recipe_once() {
    let found = parse(&Just, ".justfile", "a:\n    x\n\na:\n    y\n");
    assert_eq!(labels(&found), ["a"]);
}

#[test]
fn cargo_aliases() {
    let text = r#"
[build]
target-dir = "out"

[alias]
t = ["test", "--workspace"]
n = 3
b = "build --release"
"#;
    let found = parse(&CargoAlias, ".cargo/config.toml", text);
    assert_eq!(labels(&found), ["t", "b"]);
    assert_eq!(found[0].detail.as_deref(), Some("test --workspace"));
    assert_eq!(found[1].detail.as_deref(), Some("build --release"));
    assert_eq!(line("cargo", ".cargo/config.toml", &found[1]), "cargo b");
    assert!(parse(&CargoAlias, ".cargo/config.toml", "[build]\njobs = 2").is_empty());
}

#[test]
fn procfile_entries_are_background() {
    let text = "\
# processes
web: bundle exec puma -p $PORT
worker:   sh -c 'sleep 1; echo hi'

empty:
not a process line
";
    let found = parse(&Procfile, "Procfile", text);
    assert_eq!(labels(&found), ["web", "worker"]);
    assert_eq!(found[0].command, "bundle exec puma -p $PORT");
    assert_eq!(found[1].command, "sh -c 'sleep 1; echo hi'");
    for found in &found {
        assert!(found.background);
        assert!(found.shell);
        assert!(found.args.is_empty());
    }
}

#[test]
fn makefile_targets_skip_special_pattern_and_assignments() {
    let text = "\
CC := gcc
CFLAGS ?= -O2
OBJS = a.o b.o
VERSION:=1
.PHONY: all test
_internal:
\t@true

all: build test ## Build and test
\t@echo all

build:
\tcc -o app main.c

%.o: %.c
\t$(CC) -c $<

test : build ##   Run the suite  
\t./app --test

bin/app: main.c
\tcc -o $@ $<

test:
\t@echo again
";
    let found = parse(&Makefile, "Makefile", text);
    // `%` rules, `bin/app` (not a plain name) and the repeated `test` are out.
    assert_eq!(labels(&found), ["all", "build", "test"]);
    assert_eq!(found[0].detail.as_deref(), Some("Build and test"));
    assert_eq!(found[1].detail, None);
    assert_eq!(found[2].detail.as_deref(), Some("Run the suite"));
    assert_eq!(line("make", "Makefile", &found[0]), "make all");
}

#[test]
fn a_huge_package_json_lists_in_file_order_quickly() {
    // A repo's file is untrusted, so ordering must stay linear in its size.
    let names: Vec<String> = (0..50_000).rev().map(|n| format!("s{n}")).collect();
    let scripts: Vec<String> = names
        .iter()
        .map(|name| format!("\"{name}\": \"x\""))
        .collect();
    let text = format!("{{ \"scripts\": {{ {} }} }}", scripts.join(", "));
    let started = std::time::Instant::now();
    let found = parse(&PackageJson, "package.json", &text);
    let took = started.elapsed();
    assert_eq!(labels(&found), names);
    assert!(took < std::time::Duration::from_secs(10), "took {took:?}");
}

#[test]
fn huge_justfiles_and_makefiles_list_quickly() {
    let names: Vec<String> = (0..50_000).map(|n| format!("t{n}")).collect();
    for (source, file, body) in [
        (&Just as &dyn Source, "justfile", "    x"),
        (&Makefile, "Makefile", "\tx"),
    ] {
        let text: String = names
            .iter()
            .map(|name| format!("{name}:\n{body}\n"))
            .collect();
        let started = std::time::Instant::now();
        let found = parse(source, file, &text);
        let took = started.elapsed();
        assert_eq!(labels(&found), names);
        assert!(
            took < std::time::Duration::from_secs(10),
            "{file} took {took:?}"
        );
    }
}
