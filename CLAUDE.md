# CLAUDE.md

## Project Overview

Derive macro crate that generates C# type definitions from Rust structs and enums.
Designed for projects sharing types between a Rust backend and C#/.NET or Unity clients
via JSON serialization (serde-compatible).

## Workspace Structure

```
crates/
  csharp-rs/          # Public crate: CSharp trait + re-export of derive macro
  csharp-rs-macros/   # Proc macro crate: #[derive(CSharp)] implementation
```

## Setup

After cloning, activate the Conventional Commits hook:

```bash
git config core.hooksPath .githooks
```

## Conventions

- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/) format required
- **Language**: English for everything (code, comments, docs)
- **Edition**: 2024
- **Lints**: Workspace-level clippy pedantic + restriction subset (see root Cargo.toml)

## Design Principles

- Respect `#[serde(...)]` attributes (`rename_all`, `rename`, `skip`, `flatten`, `tag`/`content`)
- Configurable C# target version via `[package.metadata.csharp]` in consumer's Cargo.toml
- Support both `System.Text.Json` and `Newtonsoft.Json` serializer attributes
- Export mechanism via `#[csharp(export)]` (generates files at `cargo test`, like ts-rs)
