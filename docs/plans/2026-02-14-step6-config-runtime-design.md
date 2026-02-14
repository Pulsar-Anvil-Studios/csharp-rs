# Design: Step 6 — Runtime Config

**Date:** 2026-02-14
**Status:** Approved

## Context

The `csharp-rs` workspace reads configuration (namespace, serializer, C# version, export directory) from `[package.metadata.csharp]` in the consumer's `Cargo.toml` at proc-macro expansion time. This bakes all settings into the generated code, preventing runtime flexibility.

Following the ts-rs v12 pattern (`Config` struct passed as `&Config` to all trait methods), we make all configuration runtime-configurable. Users construct a `Config` with builder methods and pass it to trait calls.

## Design Decisions

- **All settings runtime** — namespace, serializer, C# version, and export directory are all configurable at runtime
- **Remove compile-time config** — `[package.metadata.csharp]` parsing and the `toml` dependency are removed entirely
- **`&Config` on all methods** — including `csharp_name()` for forward compatibility, even though type names don't currently depend on config
- **Builder pattern** — `Config::default().with_serializer(Serializer::Newtonsoft).with_target(CSharpVersion::CSharp11)`
- **Runtime branching** — the proc macro generates code with match/if branches on config values; all serializer/version variants exist in the generated output

## Architecture

### Type migrations (macro crate -> runtime crate)

`Serializer`, `CSharpVersion`, `CSharpNamespace` move from `csharp-rs-macros` to `csharp-rs` so they're available at runtime. The macro crate retains its own `CSharpNamespace` validation for `#[csharp(namespace = "...")]` attribute parsing (no circular dependency).

### Config struct

```rust
// In csharp-rs/src/lib.rs (or config module)
pub struct Config {
    namespace: CSharpNamespace,
    serializer: Serializer,
    target: CSharpVersion,
    export_dir: PathBuf,
}
```

Defaults: namespace=`"Generated"`, serializer=`SystemTextJson`, target=`CSharp9`, export_dir=`"./csharp-bindings"`.

Builder: `with_namespace()` (panics on invalid), `with_validated_namespace()` (pre-validated), `with_serializer()`, `with_target()`, `with_export_dir()`.

Getters: `namespace() -> &str`, `serializer() -> Serializer`, `target() -> CSharpVersion`, `export_dir() -> &Path`.

### CSharp trait

```rust
pub trait CSharp {
    fn csharp_name(cfg: &Config) -> String;
    fn csharp_definition(cfg: &Config) -> String;
    fn dependencies(cfg: &Config) -> Vec<String>;
    fn csharp_fields(cfg: &Config) -> Vec<CSharpFieldInfo> { Vec::new() }
}
```

### export_to function

```rust
pub fn export_to<T: CSharp>(cfg: &Config, path: impl AsRef<Path>) -> std::io::Result<()>
```

### Proc macro codegen

The macro generates code that branches on `cfg.serializer()` and `cfg.target()` at runtime:

```rust
impl csharp_rs::CSharp for MyStruct {
    fn csharp_definition(cfg: &csharp_rs::Config) -> String {
        let ns = /* #[csharp(namespace)] override or cfg.namespace() */;
        let using = match cfg.serializer() {
            csharp_rs::Serializer::SystemTextJson => "using System.Text.Json.Serialization;",
            csharp_rs::Serializer::Newtonsoft => "using Newtonsoft.Json;",
        };
        // ... runtime-branched format
    }
}
```

### DerivedCSharp IR change

`namespace: CSharpNamespace` becomes `namespace_override: Option<String>` (set only when `#[csharp(namespace = "...")]` is present).

### Export test generation

Generated `#[test]` functions use `Config::default()`:

```rust
#[test]
fn export_csharp_mystruct() {
    let cfg = csharp_rs::Config::default();
    csharp_rs::export_to::<MyStruct>(&cfg, "csharp-bindings/MyStruct.cs")
        .expect("failed to export C# definition");
}
```

### What the macro crate retains

- `CSharpNamespace` validation (for attribute parsing)
- `DerivedCSharp` IR
- `Inflection` logic (compile-time field name transforms)
- Serde attribute parsing

### What the macro crate removes

- `CSharpConfig` struct
- `from_manifest_dir()` / `from_toml_str()`
- `Serializer` and `CSharpVersion` enums (referenced only as `csharp_rs::Serializer` in generated code)
- `toml` dependency

## Testing

- All existing tests updated for `&Config` parameter
- New builder/validation tests for `Config`
- Multi-config integration tests (same type, different configs)
- Token stream tests updated for runtime branching patterns
- Target: maintain 98%+ coverage

## Verification

```bash
cargo test --workspace
cargo clippy --workspace
cargo llvm-cov --workspace --fail-under-lines 98
```
