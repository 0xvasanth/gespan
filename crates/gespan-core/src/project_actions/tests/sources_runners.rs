use super::super::sources::runners::{
    package_manager, CargoAlias, Deno, Just, Makefile, Mise, PackageJson, Procfile,
};
use super::super::sources::Source;
use super::super::tasks_json::resolve;
use super::super::{Ctx, Found};
use std::path::Path;

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
    let text = r#"{ "scripts": {
        "build": "tsc",
        "prebuild": "rm -rf dist",
        "postbuild": "echo done",
        "predeploy": "echo orphan hook, no base script",
        "pre": "echo a script that is only named pre",
        "test": "vitest",
        "notascript": 5
    } }"#;
    let found = parse_with("pnpm", &PackageJson, "package.json", text);
    assert_eq!(labels(&found), ["build", "pre", "predeploy", "test"]);
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
    // serde_json keeps keys sorted, so the names come out alphabetically.
    assert_eq!(labels(&found), ["all", "check", "dev"]);
    assert_eq!(found[0].detail, None);
    assert_eq!(found[1].detail.as_deref(), Some("Type-check"));
    assert_eq!(found[2].detail.as_deref(), Some("deno run -A main.ts"));
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
    assert_eq!(labels(&found), ["build", "fmt", "lint", "test:unit"]);
    assert_eq!(found[0].detail.as_deref(), Some("Build it"));
    assert_eq!(found[3].detail, None);
    assert!(found[2].hide);
    assert!(!found[0].hide);
    assert_eq!(line("mise", "mise.toml", &found[3]), "mise run test:unit");
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
    assert_eq!(labels(&found), ["build", "quiet", "deploy", "tidy"]);
    assert_eq!(found[0].detail.as_deref(), Some("Build everything"));
    // `test target=.all.:` has a default argument, which the recipe pattern
    // skips, and its doc comment must not carry over to `quiet`.
    assert_eq!(found[1].detail, None);
    assert_eq!(found[2].detail, None);
    assert_eq!(line("just", "justfile", &found[0]), "just build");
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
b = "build --release"
t = ["test", "--workspace"]
n = 3
"#;
    let found = parse(&CargoAlias, ".cargo/config.toml", text);
    assert_eq!(labels(&found), ["b", "t"]);
    assert_eq!(found[0].detail.as_deref(), Some("build --release"));
    assert_eq!(found[1].detail.as_deref(), Some("test --workspace"));
    assert_eq!(line("cargo", ".cargo/config.toml", &found[0]), "cargo b");
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
