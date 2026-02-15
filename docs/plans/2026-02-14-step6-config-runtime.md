# Step 6: Runtime Config — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> **REQUIRED SKILL:** Use `ms-rust` skill BEFORE editing any `.rs` file.

**Goal:** Migrate all configuration (namespace, serializer, C# version, export directory) from compile-time (`[package.metadata.csharp]` in Cargo.toml) to runtime (`&Config` passed to all `CSharp` trait methods), following the ts-rs v12 pattern.

**Architecture:** Move `Serializer`, `CSharpVersion`, `CSharpNamespace` types to the runtime crate (`csharp-rs`). Add a `Config` struct with builder pattern. Add `cfg: &Config` parameter to all `CSharp` trait methods and `export_to`. Update the proc macro to generate runtime-branching code instead of compile-time decisions. Remove `toml` dependency and `[package.metadata.csharp]` support entirely.

**Tech Stack:** Rust 2024 edition, `syn`/`quote`/`proc-macro2`, strict clippy pedantic lints. GPG signing unavailable — use `git commit --no-gpg-sign`.

---

## Task 1: Add `Serializer` and `CSharpVersion` to runtime crate

**Files:**
- Modify: `crates/csharp-rs/src/lib.rs:23-28` (add types after imports, before `CSharpFieldInfo`)
- Test: unit tests in same file

These types are public API of the runtime crate. The macro crate will reference them as `csharp_rs::Serializer` in generated code.

**Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/csharp-rs/src/lib.rs` (after line 207, before existing tests):

```rust
#[test]
fn serializer_default_is_system_text_json() {
    assert_eq!(Serializer::default(), Serializer::SystemTextJson);
}

#[test]
fn csharp_version_default_is_csharp9() {
    assert_eq!(CSharpVersion::default(), CSharpVersion::CSharp9);
}

#[test]
fn csharp_version_ordering() {
    assert!(CSharpVersion::CSharp9 < CSharpVersion::CSharp10);
    assert!(CSharpVersion::CSharp10 < CSharpVersion::CSharp11);
    assert!(CSharpVersion::CSharp11 < CSharpVersion::CSharp12);
}

#[test]
fn csharp_version_display() {
    assert_eq!(CSharpVersion::CSharp9.to_string(), "9.0");
    assert_eq!(CSharpVersion::CSharp10.to_string(), "10.0");
    assert_eq!(CSharpVersion::CSharp11.to_string(), "11.0");
    assert_eq!(CSharpVersion::CSharp12.to_string(), "12.0");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p csharp-rs tests`
Expected: FAIL — `Serializer`, `CSharpVersion` don't exist in the runtime crate.

**Step 3: Implement the types**

In `crates/csharp-rs/src/lib.rs`, add after the `use` imports (after line 24) and before `CSharpFieldInfo` (before line 32):

```rust
// ---------------------------------------------------------------------------
// Configuration enums
// ---------------------------------------------------------------------------

/// Which JSON serializer library to target in generated C# code.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Serializer {
    /// `System.Text.Json` attributes (default).
    #[default]
    SystemTextJson,
    /// `Newtonsoft.Json` attributes.
    Newtonsoft,
}

/// Target C# language version — controls which syntax features are used.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CSharpVersion {
    /// C# 9.0 (default) — positional records, block-scoped namespaces.
    #[default]
    CSharp9,
    /// C# 10.0 — file-scoped namespaces.
    CSharp10,
    /// C# 11.0 — `required` modifier, native `[JsonPolymorphic]`.
    CSharp11,
    /// C# 12.0 — primary constructors.
    CSharp12,
}

impl std::fmt::Display for CSharpVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::CSharp9 => "9.0",
            Self::CSharp10 => "10.0",
            Self::CSharp11 => "11.0",
            Self::CSharp12 => "12.0",
        };
        f.write_str(s)
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p csharp-rs tests`
Expected: ALL PASS

**Step 5: Run full workspace tests**

Run: `cargo fmt --workspace && cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 6: Commit**

```
feat(csharp-rs): add Serializer and CSharpVersion enums to runtime crate
```

---

## Task 2: Add `CSharpNamespace` to runtime crate

**Files:**
- Modify: `crates/csharp-rs/src/lib.rs` (add after `CSharpVersion` Display impl)

The validation logic is copied from `crates/csharp-rs-macros/src/config.rs:69-127`. The macro crate retains its own copy for attribute parsing (no circular dependency).

**Step 1: Write failing tests**

Add to `#[cfg(test)] mod tests` in `crates/csharp-rs/src/lib.rs`:

```rust
#[test]
fn namespace_valid_single_segment() {
    let ns = CSharpNamespace::new("MyGame").unwrap();
    assert_eq!(ns.as_ref(), "MyGame");
}

#[test]
fn namespace_valid_multi_segment() {
    let ns = CSharpNamespace::new("Company.Product.Module").unwrap();
    assert_eq!(ns.as_ref(), "Company.Product.Module");
}

#[test]
fn namespace_underscore_prefix_valid() {
    assert!(CSharpNamespace::new("_Internal").is_ok());
}

#[test]
fn namespace_invalid_empty() {
    assert!(CSharpNamespace::new("").is_err());
}

#[test]
fn namespace_invalid_starts_with_digit() {
    assert!(CSharpNamespace::new("1Invalid").is_err());
}

#[test]
fn namespace_invalid_special_chars() {
    assert!(CSharpNamespace::new("My-Namespace").is_err());
}

#[test]
fn namespace_invalid_empty_segment() {
    assert!(CSharpNamespace::new("A..B").is_err());
}

#[test]
fn namespace_display() {
    let ns = CSharpNamespace::new("Test.Ns").unwrap();
    assert_eq!(ns.to_string(), "Test.Ns");
}

#[test]
fn namespace_partial_eq_str() {
    let ns = CSharpNamespace::new("Generated").unwrap();
    assert_eq!(ns, "Generated");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p csharp-rs tests`
Expected: FAIL — `CSharpNamespace` doesn't exist in runtime crate.

**Step 3: Implement**

In `crates/csharp-rs/src/lib.rs`, add after the `CSharpVersion` Display impl (before `CSharpFieldInfo`):

```rust
/// A validated C# namespace (e.g. `"Company.Product"`).
///
/// Each segment must start with an ASCII letter or underscore and contain
/// only ASCII alphanumeric characters or underscores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSharpNamespace(String);

impl CSharpNamespace {
    /// Creates a new validated namespace.
    ///
    /// # Errors
    ///
    /// Returns an error message if the namespace is empty, contains empty
    /// segments, or has segments with invalid characters.
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let s = value.into();
        validate_namespace(&s)?;
        Ok(Self(s))
    }
}

impl std::fmt::Display for CSharpNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CSharpNamespace {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<&str> for CSharpNamespace {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Validates a C# namespace string.
fn validate_namespace(ns: &str) -> Result<(), &'static str> {
    if ns.is_empty() {
        return Err("namespace must not be empty");
    }
    for segment in ns.split('.') {
        if segment.is_empty() {
            return Err("namespace must not contain empty segments");
        }
        let mut chars = segment.chars();
        let first = chars.next().expect("segment is non-empty");
        if !first.is_ascii_alphabetic() && first != '_' {
            return Err("each segment must start with a letter or underscore");
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("segments must contain only letters, digits, or underscores");
        }
    }
    Ok(())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p csharp-rs tests`
Expected: ALL PASS

**Step 5: Run full workspace tests**

Run: `cargo fmt --workspace && cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 6: Commit**

```
feat(csharp-rs): add CSharpNamespace validated type to runtime crate
```

---

## Task 3: Add `Config` struct with builder pattern

**Files:**
- Modify: `crates/csharp-rs/src/lib.rs` (add after `CSharpNamespace`)
- Modify: `crates/csharp-rs/Cargo.toml` (no changes needed — `std::path::PathBuf` is in std)

**Step 1: Write failing tests**

Add to `#[cfg(test)] mod tests` in `crates/csharp-rs/src/lib.rs`:

```rust
#[test]
fn config_default_values() {
    let cfg = Config::default();
    assert_eq!(cfg.namespace(), "Generated");
    assert_eq!(cfg.serializer(), Serializer::SystemTextJson);
    assert_eq!(cfg.target(), CSharpVersion::CSharp9);
    assert_eq!(cfg.export_dir(), Path::new("./csharp-bindings"));
}

#[test]
fn config_with_serializer() {
    let cfg = Config::default().with_serializer(Serializer::Newtonsoft);
    assert_eq!(cfg.serializer(), Serializer::Newtonsoft);
}

#[test]
fn config_with_target() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp12);
    assert_eq!(cfg.target(), CSharpVersion::CSharp12);
}

#[test]
fn config_with_namespace() {
    let cfg = Config::default().with_namespace("My.Game");
    assert_eq!(cfg.namespace(), "My.Game");
}

#[test]
#[should_panic(expected = "each segment must start with a letter")]
fn config_with_namespace_panics_on_invalid() {
    let _ = Config::default().with_namespace("1Bad");
}

#[test]
fn config_with_validated_namespace() {
    let ns = CSharpNamespace::new("Pre.Validated").unwrap();
    let cfg = Config::default().with_validated_namespace(ns);
    assert_eq!(cfg.namespace(), "Pre.Validated");
}

#[test]
fn config_with_export_dir() {
    let cfg = Config::default().with_export_dir("./output");
    assert_eq!(cfg.export_dir(), Path::new("./output"));
}

#[test]
fn config_builder_chaining() {
    let cfg = Config::default()
        .with_namespace("Unity.Types")
        .with_serializer(Serializer::Newtonsoft)
        .with_target(CSharpVersion::CSharp11)
        .with_export_dir("./generated");
    assert_eq!(cfg.namespace(), "Unity.Types");
    assert_eq!(cfg.serializer(), Serializer::Newtonsoft);
    assert_eq!(cfg.target(), CSharpVersion::CSharp11);
    assert_eq!(cfg.export_dir(), Path::new("./generated"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p csharp-rs tests`
Expected: FAIL — `Config` doesn't exist.

**Step 3: Implement**

Add `use std::path::PathBuf;` to the imports at the top of `crates/csharp-rs/src/lib.rs` (line 24).

Add after `CSharpNamespace` and `validate_namespace` (before `CSharpFieldInfo`):

```rust
// ---------------------------------------------------------------------------
// Runtime configuration
// ---------------------------------------------------------------------------

/// Runtime configuration for C# code generation.
///
/// Controls namespace, serializer library, C# language version, and export
/// directory. Construct with [`Config::default`] and customize with builder
/// methods.
///
/// # Examples
///
/// ```
/// use csharp_rs::{Config, Serializer, CSharpVersion};
///
/// let cfg = Config::default()
///     .with_serializer(Serializer::Newtonsoft)
///     .with_target(CSharpVersion::CSharp11);
/// ```
pub struct Config {
    namespace: CSharpNamespace,
    serializer: Serializer,
    target: CSharpVersion,
    export_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            namespace: CSharpNamespace::new("Generated")
                .expect("default namespace is valid"),
            serializer: Serializer::SystemTextJson,
            target: CSharpVersion::CSharp9,
            export_dir: PathBuf::from("./csharp-bindings"),
        }
    }
}

impl Config {
    /// Sets the root namespace. Panics if the value is not a valid C#
    /// namespace.
    ///
    /// # Panics
    ///
    /// Panics if `ns` fails [`CSharpNamespace`] validation.
    #[must_use]
    pub fn with_namespace(mut self, ns: &str) -> Self {
        self.namespace = CSharpNamespace::new(ns)
            .unwrap_or_else(|e| panic!("invalid namespace \"{ns}\": {e}"));
        self
    }

    /// Sets the root namespace from a pre-validated [`CSharpNamespace`].
    #[must_use]
    pub fn with_validated_namespace(mut self, ns: CSharpNamespace) -> Self {
        self.namespace = ns;
        self
    }

    /// Sets the target serializer library.
    #[must_use]
    pub fn with_serializer(mut self, serializer: Serializer) -> Self {
        self.serializer = serializer;
        self
    }

    /// Sets the target C# language version.
    #[must_use]
    pub fn with_target(mut self, target: CSharpVersion) -> Self {
        self.target = target;
        self
    }

    /// Sets the export directory for generated `.cs` files.
    #[must_use]
    pub fn with_export_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.export_dir = dir.into();
        self
    }

    /// Returns the configured namespace as a string slice.
    pub fn namespace(&self) -> &str {
        self.namespace.as_ref()
    }

    /// Returns the configured serializer.
    pub fn serializer(&self) -> Serializer {
        self.serializer
    }

    /// Returns the configured C# target version.
    pub fn target(&self) -> CSharpVersion {
        self.target
    }

    /// Returns the configured export directory.
    pub fn export_dir(&self) -> &Path {
        &self.export_dir
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p csharp-rs tests`
Expected: ALL PASS

**Step 5: Run full workspace tests**

Run: `cargo fmt --workspace && cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 6: Commit**

```
feat(csharp-rs): add Config struct with builder pattern for runtime configuration
```

---

## Task 4: Add `cfg: &Config` to `CSharp` trait methods and `export_to`

This is the breaking change to the public API. All existing trait impls (primitives, generics) and the `export_to` function must be updated. The derive macro output will be updated in a later task.

**Files:**
- Modify: `crates/csharp-rs/src/lib.rs:57-203` (trait definition, `export_to`, primitive macro, generic impls)
- Modify: `crates/csharp-rs/src/lib.rs` tests section (update all test calls)

**Step 1: Update the trait definition**

In `crates/csharp-rs/src/lib.rs`, change the `CSharp` trait (lines 57-75) to:

```rust
pub trait CSharp {
    /// Returns the C# type name for this Rust type (e.g. `"int"`, `"MyStruct"`).
    fn csharp_name(cfg: &Config) -> String;

    /// Returns the complete `.cs` file content for this type, or empty for
    /// primitives / generics.
    fn csharp_definition(cfg: &Config) -> String;

    /// Returns C# type names this type depends on (for transitive export).
    fn dependencies(cfg: &Config) -> Vec<String>;

    /// Returns metadata about this type's fields (used by `#[serde(flatten)]`).
    fn csharp_fields(_cfg: &Config) -> Vec<CSharpFieldInfo> {
        Vec::new()
    }
}
```

**Step 2: Update `export_to`**

Change `export_to` (lines 84-90) to:

```rust
pub fn export_to<T: CSharp>(cfg: &Config, path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, T::csharp_definition(cfg))
}
```

**Step 3: Update the primitive macro**

Change `impl_csharp_primitive!` (lines 96-113) to:

```rust
macro_rules! impl_csharp_primitive {
    ($rust_ty:ty, $csharp_name:expr) => {
        impl CSharp for $rust_ty {
            fn csharp_name(_cfg: &Config) -> String {
                String::from($csharp_name)
            }

            fn csharp_definition(_cfg: &Config) -> String {
                // Primitives have no standalone definition.
                String::new()
            }

            fn dependencies(_cfg: &Config) -> Vec<String> {
                Vec::new()
            }
        }
    };
}
```

**Step 4: Update generic impls**

Change `Option<T>` impl (lines 149-161) to:

```rust
impl<T: CSharp> CSharp for Option<T> {
    fn csharp_name(cfg: &Config) -> String {
        // Options unwrap to the inner type — nullability is expressed at the
        // property level (with `?` suffix), not in the type name.
        T::csharp_name(cfg)
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![T::csharp_name(cfg)]
    }
}
```

Change `Vec<T>` impl (lines 163-175) to:

```rust
impl<T: CSharp> CSharp for Vec<T> {
    fn csharp_name(cfg: &Config) -> String {
        format!("List<{}>", T::csharp_name(cfg))
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![T::csharp_name(cfg)]
    }
}
```

Change `HashMap<K, V, S>` impl (lines 177-189) to:

```rust
impl<K: CSharp, V: CSharp, S> CSharp for HashMap<K, V, S> {
    fn csharp_name(cfg: &Config) -> String {
        format!("Dictionary<{}, {}>", K::csharp_name(cfg), V::csharp_name(cfg))
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![K::csharp_name(cfg), V::csharp_name(cfg)]
    }
}
```

Change `HashSet<T, S>` impl (lines 191-203) to:

```rust
impl<T: CSharp, S> CSharp for HashSet<T, S> {
    fn csharp_name(cfg: &Config) -> String {
        format!("HashSet<{}>", T::csharp_name(cfg))
    }

    fn csharp_definition(_cfg: &Config) -> String {
        String::new()
    }

    fn dependencies(cfg: &Config) -> Vec<String> {
        vec![T::csharp_name(cfg)]
    }
}
```

**Step 5: Update all tests in the runtime crate**

Every existing test that calls `T::csharp_name()`, `T::csharp_definition()`, `T::dependencies()`, `T::csharp_fields()`, or `export_to::<T>(path)` must add a `&Config::default()` argument:

- Pattern: `String::csharp_name()` → `String::csharp_name(&Config::default())`
- Pattern: `String::csharp_definition()` → `String::csharp_definition(&Config::default())`
- Pattern: `String::dependencies()` → `String::dependencies(&Config::default())`
- Pattern: `String::csharp_fields()` → `String::csharp_fields(&Config::default())`
- Pattern: `export_to::<T>(path)` → `export_to::<T>(&Config::default(), path)`

Apply to **every test** in the `mod tests` block (lines 206-371). Create a `let cfg = Config::default();` at the top of each test that calls multiple methods.

Example transformations:

```rust
// Before:
#[test]
fn string_maps_to_csharp_string() {
    assert_eq!(String::csharp_name(), "string");
}

// After:
#[test]
fn string_maps_to_csharp_string() {
    let cfg = Config::default();
    assert_eq!(String::csharp_name(&cfg), "string");
}
```

```rust
// Before:
#[test]
fn export_to_writes_file() {
    // ... setup ...
    export_to::<SimpleExportStruct>(path.as_path()).unwrap();
    // ...
}

// After:
#[test]
fn export_to_writes_file() {
    let cfg = Config::default();
    // ... setup ...
    export_to::<SimpleExportStruct>(&cfg, path.as_path()).unwrap();
    // ...
}
```

**Step 6: Run tests (runtime crate only compiles at this point)**

Run: `cargo test -p csharp-rs`
Expected: ALL PASS (runtime crate tests pass; workspace will fail because generated code from proc macro still uses the old signature — that's expected and fixed in later tasks).

**Step 7: Commit**

```
feat(csharp-rs)!: add cfg parameter to CSharp trait methods and export_to
```

---

## Task 5: Update proc macro codegen — trait impl signature

The proc macro must generate code with the new `cfg: &csharp_rs::Config` parameter on all trait methods. The definition/dependencies/fields bodies still use compile-time config internally for now — runtime branching is Task 8+. The critical change is the method signatures.

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs:54-84` (`into_token_stream`)
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs:244-267` (`build_export_test`)
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs:118-191` (`build_dependencies`)
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs:270-327` (`build_csharp_fields`)

**Step 1: Update `into_token_stream`**

In `crates/csharp-rs-macros/src/codegen/mod.rs`, change `into_token_stream` (lines 54-84) to:

```rust
    pub fn into_token_stream(self, config: &CSharpConfig) -> TokenStream {
        let ident = &self.rust_ident;
        let csharp_name = &self.csharp_name;

        let definition_body = self.build_definition(config);
        let dependencies_body = self.build_dependencies();
        let csharp_fields_body = self.build_csharp_fields();
        let export_test = self.build_export_test(config);

        quote! {
            impl csharp_rs::CSharp for #ident {
                fn csharp_name(_cfg: &csharp_rs::Config) -> String {
                    String::from(#csharp_name)
                }

                fn csharp_definition(cfg: &csharp_rs::Config) -> String {
                    let _ = cfg; // TODO: use for runtime branching in later tasks
                    #definition_body
                }

                fn dependencies(cfg: &csharp_rs::Config) -> Vec<String> {
                    let _ = cfg; // TODO: use for runtime branching in later tasks
                    #dependencies_body
                }

                fn csharp_fields(cfg: &csharp_rs::Config) -> Vec<csharp_rs::CSharpFieldInfo> {
                    let _ = cfg; // TODO: use for runtime branching in later tasks
                    #csharp_fields_body
                }
            }

            #export_test
        }
    }
```

Note: `cfg` is bound but intentionally silenced with `let _ = cfg;` until Tasks 8-10 introduce runtime branching. `csharp_name` doesn't use config, so it uses `_cfg`.

**Step 2: Update `build_export_test`**

Change `build_export_test` (lines 244-267) to generate code using the new `export_to` signature:

```rust
    fn build_export_test(&self, config: &CSharpConfig) -> TokenStream {
        if !self.export {
            return TokenStream::new();
        }

        let ident = &self.rust_ident;
        let fn_name = quote::format_ident!(
            "export_csharp_{}",
            ident.to_string().to_ascii_lowercase()
        );

        let file_name = self.export_to.clone().unwrap_or_else(|| {
            let dir = config.export_dir.display();
            format!("{dir}/{}.cs", self.csharp_name)
        });

        quote! {
            #[test]
            fn #fn_name() {
                let cfg = csharp_rs::Config::default();
                csharp_rs::export_to::<#ident>(&cfg, #file_name)
                    .expect("failed to export C# definition");
            }
        }
    }
```

**Step 3: Update `build_dependencies` — pass `cfg` to inner type calls**

In `build_dependencies` (lines 118-191), every call to `<T as csharp_rs::CSharp>::csharp_name()` must become `<T as csharp_rs::CSharp>::csharp_name(cfg)`. This means the `type_expr` token streams in the IR already contain expressions like `<String as csharp_rs::CSharp>::csharp_name()`. These are generated in field processing.

The dependencies function body already emits token streams that call `csharp_name()`. These calls are embedded in the generated code and need `cfg` passed. The issue is that `type_expr` in `CSharpField` stores token fragments like `<String as csharp_rs::CSharp>::csharp_name()`.

Search for where `type_expr` is created (in `types/named.rs` and `types/tagged_enum.rs`) — these build expressions like:
```rust
quote! { <#ty as csharp_rs::CSharp>::csharp_name() }
```

These must become:
```rust
quote! { <#ty as csharp_rs::CSharp>::csharp_name(cfg) }
```

The `build_dependencies` body generates code that runs at runtime. Since the trait method now receives `cfg`, and the generated code has access to `cfg`, the `type_expr` just needs to pass `cfg` through.

Update in `crates/csharp-rs-macros/src/types/named.rs`, the type expression construction (around line 98):

Find: `quote! { <#ty as csharp_rs::CSharp>::csharp_name() }`
Replace with: `quote! { <#ty as csharp_rs::CSharp>::csharp_name(cfg) }`

Similarly update in `crates/csharp-rs-macros/src/types/tagged_enum.rs` wherever `type_expr` is built.

**Step 4: Update `build_csharp_fields` — pass `cfg` to inner type calls**

In `build_csharp_fields` (lines 270-327), the generated code calls `<T as csharp_rs::CSharp>::csharp_fields()` for flattened types. These must become `<T as csharp_rs::CSharp>::csharp_fields(cfg)`.

Find all `csharp_fields()` calls in the generated token streams and add `cfg`.

Also in `build_field_dependencies` (lines 198-241), `<T as csharp_rs::CSharp>::csharp_fields()` calls must gain `cfg`.

**Step 5: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS — the generated code now compiles against the new trait signature.

**Step 6: Run full checks**

Run: `cargo fmt --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 7: Commit**

```
feat(macro)!: generate CSharp trait impls with cfg parameter
```

---

## Task 6: Update integration tests for `&Config`

The integration tests in `crates/csharp-rs/tests/` call trait methods directly. They need `&Config::default()`.

**Files:**
- Modify: `crates/csharp-rs/tests/derive_struct.rs`
- Modify: `crates/csharp-rs/tests/derive_enum.rs`
- Modify: `crates/csharp-rs/tests/derive_tagged_enum.rs`

**Step 1: Update all integration tests**

In each file, add `use csharp_rs::Config;` to imports, and change every trait method call:

- `T::csharp_name()` → `T::csharp_name(&Config::default())`
- `T::csharp_definition()` → `T::csharp_definition(&Config::default())`
- `T::dependencies()` → `T::dependencies(&Config::default())`
- `T::csharp_fields()` → `T::csharp_fields(&Config::default())`

For readability, each test function should start with `let cfg = Config::default();` and use `&cfg` everywhere.

**Step 2: Run integration tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 3: Commit**

```
test(integration): update derive tests for Config parameter
```

---

## Task 7: Update macro crate codegen tests for `&Config`

The codegen unit tests in `crates/csharp-rs-macros/src/codegen/mod.rs` (lines 329-2277) check the generated token stream text. These tests check for string patterns in the `into_token_stream()` output. The generated code now has `cfg : & csharp_rs :: Config` in method signatures and `let _ = cfg ;` lines.

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs` (tests section, lines 329+)

**Step 1: Identify affected assertions**

The tests use `tokens.contains(...)` to check patterns. The new signatures will produce tokens like:
- `fn csharp_name (_cfg : & csharp_rs :: Config)` (note: proc-macro2 adds spaces around punctuation)
- `fn csharp_definition (cfg : & csharp_rs :: Config)`
- `csharp_rs :: export_to :: < #ident > (& cfg ,` in export tests
- `csharp_rs :: Config :: default ()` in export tests

Any test that checks for the old `fn csharp_name ()` signature will still match because it checks for content *within* the definition, not the signature itself.

Review each test and update assertions that specifically check for:
1. Export test patterns (the export test generation changed significantly)
2. Any assertion checking method signatures

Most tests check for string content within `csharp_definition()` return values (like `"JsonPropertyName"`, `"public sealed record"`, etc.), which haven't changed. The main ones that need updating are the export test assertions.

**Step 2: Update export test assertions**

Find tests that assert on `export_to` patterns and update them to expect `Config::default()` and `& cfg`:

```rust
// Tests like `export_generates_test_fn` should now check for:
assert!(tokens.contains("Config :: default ()"),
    "export test should use Config::default():\n{tokens}");
assert!(tokens.contains("export_to"),
    "export test should call export_to:\n{tokens}");
```

**Step 3: Run tests**

Run: `cargo test -p csharp-rs-macros codegen::tests`
Expected: ALL PASS

**Step 4: Run full workspace**

Run: `cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 5: Commit**

```
test(macro): update codegen tests for Config parameter in generated code
```

---

## Task 8: Change `DerivedCSharp.namespace` to `namespace_override: Option<String>`

Currently the IR stores the resolved namespace (either from `#[csharp(namespace)]` or the config default). Since config is now runtime, the IR should only store the override, and the generated code should fall back to `cfg.namespace()` at runtime.

**Files:**
- Modify: `crates/csharp-rs-macros/src/types/mod.rs:126-140` (`DerivedCSharp` struct)
- Modify: `crates/csharp-rs-macros/src/types/named.rs:34-38` (namespace resolution)
- Modify: `crates/csharp-rs-macros/src/types/simple_enum.rs` (namespace resolution)
- Modify: `crates/csharp-rs-macros/src/types/tagged_enum.rs` (namespace resolution)
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs:87-108` (`build_definition`)
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs` (tests — `sample_ir` helpers)

**Step 1: Update `DerivedCSharp` struct**

In `crates/csharp-rs-macros/src/types/mod.rs`, change `DerivedCSharp` (lines 126-140):

```rust
pub struct DerivedCSharp {
    pub rust_ident: Ident,
    pub csharp_name: String,
    /// Per-type namespace override from `#[csharp(namespace = "...")]`.
    /// When `None`, the generated code falls back to `cfg.namespace()`.
    pub namespace_override: Option<String>,
    pub kind: DerivedCSharpKind,
    pub export: bool,
    pub export_to: Option<String>,
}
```

**Step 2: Update namespace resolution in `named_struct`**

In `crates/csharp-rs-macros/src/types/named.rs`, change the namespace logic (lines 34-38):

```rust
// Before:
let namespace = match &container.namespace {
    Some(ns) => CSharpNamespace::new(ns.as_str())
        .expect("namespace was validated in ContainerAttr::parse_csharp"),
    None => config.namespace.clone(),
};

// After:
let namespace_override = container.namespace.clone();
```

And in the return struct, change `namespace: namespace,` to `namespace_override,`.

Remove the `config` parameter from `named_struct` since it's no longer needed there (the only use was namespace fallback). Update the call site in `process_input` accordingly.

Wait — `config` is also passed through to other places. Let me check. Looking at `named_struct` signature at line 22-27: the only use of `config` is the namespace fallback. If that's removed, `config` can be dropped from the signature.

Actually, check if `process_input` passes `config` to `simple_enum` or `tagged_enum` too. Looking at `process_input` lines 147-189: yes, `config` is passed to `named_struct` at line 154. For simple_enum and tagged_enum, similar namespace resolution happens.

Update all three: `named_struct`, `simple_enum`, `tagged_enum` to set `namespace_override: container.namespace.clone()` instead of resolving against config.

**Step 3: Update `build_definition` to generate runtime namespace resolution**

In `crates/csharp-rs-macros/src/codegen/mod.rs`, change `build_definition` (lines 87-108):

```rust
    fn build_definition(&self, config: &CSharpConfig) -> TokenStream {
        // Namespace is resolved at runtime: per-type override or cfg.namespace()
        let ns_expr = match &self.namespace_override {
            Some(ns) => quote! { #ns },
            None => quote! { cfg.namespace() },
        };

        let csharp_name = &self.csharp_name;

        match &self.kind {
            DerivedCSharpKind::Record(fields) => {
                record::build_record_definition(csharp_name, &ns_expr, fields, config)
            }
            DerivedCSharpKind::Enum(variants) => {
                simple_enum::build_enum_definition(csharp_name, &ns_expr, variants, config)
            }
            DerivedCSharpKind::TaggedEnum { tagging, variants } => {
                tagged_enum::build_tagged_enum_definition(
                    csharp_name,
                    &ns_expr,
                    tagging,
                    variants,
                    config,
                )
            }
        }
    }
```

This means the `namespace` parameter in `build_record_definition`, `build_enum_definition`, and `build_tagged_enum_definition` changes from `&str` to `&TokenStream` — the expression that produces the namespace string at runtime.

**Step 4: Update codegen functions to accept namespace as `TokenStream`**

In `record.rs`, change:
```rust
pub fn build_record_definition(
    csharp_name: &str,
    ns_expr: &TokenStream,  // was: namespace: &str
    fields: &[CSharpField],
    config: &CSharpConfig,
) -> TokenStream
```

The namespace was previously interpolated as a string literal in format strings. Now it must be interpolated as a runtime expression. This requires changing the codegen approach: instead of building a single format string at compile time, build a token stream that constructs the string at runtime.

This is a significant refactor — all the `format!` calls that embed namespace need to become runtime `format!` calls in the generated code. Example:

```rust
// Before (compile-time string):
let definition = format!("namespace {namespace};\n\npublic sealed record ...");
quote! { String::from(#definition) }

// After (runtime expression):
quote! {
    {
        let ns: &str = #ns_expr;
        format!("namespace {ns};\n\npublic sealed record ...")
    }
}
```

Apply this pattern to `build_record_definition`, `build_enum_definition`, `build_tagged_enum_definition`, and all their helper functions that use namespace.

**Step 5: Update test helpers in codegen tests**

Update `sample_ir`, `sample_enum_ir`, etc. to use `namespace_override`:

```rust
fn sample_ir(export: bool, export_to: Option<String>) -> DerivedCSharp {
    DerivedCSharp {
        rust_ident: quote::format_ident!("TestStruct"),
        csharp_name: String::from("TestStruct"),
        namespace_override: Some(String::from("Test.Ns")),
        kind: DerivedCSharpKind::Record(vec![/* ... */]),
        export,
        export_to,
    }
}
```

**Step 6: Update `types/mod.rs` tests**

The `process_input` tests construct IR and check `ir.namespace`. Change to check `ir.namespace_override`:

```rust
// Before:
assert_eq!(ir.namespace, "Generated");
// After:
assert!(ir.namespace_override.is_none());
```

And for tests with `#[csharp(namespace = "Custom")]`:
```rust
// Before:
assert_eq!(ir.namespace, "Custom");
// After:
assert_eq!(ir.namespace_override.as_deref(), Some("Custom"));
```

**Step 7: Remove `config` parameter from `process_input` if no longer needed**

If `process_input` no longer needs `config` (because namespace fallback moved to codegen), remove it:

```rust
// Before:
pub fn process_input(input: &DeriveInput, config: &CSharpConfig) -> syn::Result<DerivedCSharp>

// After:
pub fn process_input(input: &DeriveInput) -> syn::Result<DerivedCSharp>
```

Update the call site in `crates/csharp-rs-macros/src/lib.rs:40`.

**Step 8: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 9: Run full checks**

Run: `cargo fmt --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 10: Commit**

```
refactor(macro): replace namespace field with namespace_override for runtime resolution
```

---

## Task 9: Runtime branching for serializer in record codegen

Replace compile-time serializer selection with runtime `match` in generated code.

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/record.rs`
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs` (pass config differently)

Currently `build_record_definition` reads `config.serializer` at macro expansion time and embeds only the matching string literals. After this change, the generated code includes a `match cfg.serializer()` and both branches.

**Step 1: Update `build_record_definition`**

The function currently accepts `config: &CSharpConfig` and uses it for compile-time decisions. Change it to NOT use `config` for serializer — instead, generate code that branches at runtime.

Remove the `config` parameter (serializer/version decisions now happen in generated code). Instead, generate all branches:

```rust
pub fn build_record_definition(
    csharp_name: &str,
    ns_expr: &TokenStream,
    fields: &[CSharpField],
) -> TokenStream
```

The generated code should look like:

```rust
{
    let ns: &str = /* ns_expr */;
    let use_file_scoped = cfg.target() >= csharp_rs::CSharpVersion::CSharp10;
    let use_required = cfg.target() >= csharp_rs::CSharpVersion::CSharp11;

    let using_directive = match cfg.serializer() {
        csharp_rs::Serializer::SystemTextJson => "using System.Text.Json.Serialization;",
        csharp_rs::Serializer::Newtonsoft => "using Newtonsoft.Json;",
    };

    // ... build fields with runtime serializer branching ...
    // ... build definition string ...
}
```

This is a significant change to how the codegen works. The field attribute selection (`JsonPropertyName` vs `JsonProperty`) must also branch at runtime.

For field expressions, currently `build_field_exprs` picks one attribute name at compile time. Change to generate code that does:

```rust
let attr_name = match cfg.serializer() {
    csharp_rs::Serializer::SystemTextJson => "JsonPropertyName",
    csharp_rs::Serializer::Newtonsoft => "JsonProperty",
};
```

The challenge is that this involves building format strings at runtime in the generated code. The cleanest approach: have the generated code build the definition string piece by piece using `format!` / `String::push_str`.

**Step 2: Update codegen tests**

The codegen tests currently check for specific serializer output (e.g., `tokens.contains("JsonPropertyName")`). After this change, the generated code contains BOTH serializer paths. Update tests to check that both paths exist in the token stream:

```rust
#[test]
fn record_token_stream_contains_both_serializer_paths() {
    let ir = sample_ir(false, None);
    let tokens = ir.into_token_stream().to_string();
    assert!(tokens.contains("JsonPropertyName"), "should contain STJ path");
    assert!(tokens.contains("JsonProperty"), "should contain Newtonsoft path");
}
```

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 4: Commit**

```
feat(macro): generate runtime serializer branching for record codegen
```

---

## Task 10: Runtime branching for serializer in simple enum codegen

Same pattern as Task 9 but for `build_enum_definition`.

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/simple_enum.rs`

**Step 1: Update `build_enum_definition`**

Remove `config` parameter. Generate code that branches on `cfg.serializer()` and `cfg.target()` at runtime. Both the converter attribute and using directives must be selected at runtime.

**Step 2: Update codegen tests for enums**

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 4: Commit**

```
feat(macro): generate runtime serializer branching for simple enum codegen
```

---

## Task 11: Runtime branching for serializer in tagged enum codegen

Same pattern for `build_tagged_enum_definition`. This is the most complex because it has 4 tagging modes × 2 serializers × C# version branching, plus native polymorphism for Internal + STJ + C# 11+.

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/tagged_enum.rs`

**Step 1: Update `build_tagged_enum_definition`**

Remove `config` parameter. Generate code that branches on `cfg.serializer()` and `cfg.target()` at runtime. The native polymorphism check `use_native_polymorphism` must also become a runtime check.

The converter generation, using block, class attributes, and variant expressions all need runtime branching.

**Step 2: Update tagged enum codegen tests**

The extensive test suite (many tests in `codegen/mod.rs` lines 329-2277) needs updating. Tests that check for STJ-specific output should now check that both paths exist, or run the generated code against a specific config.

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 4: Commit**

```
feat(macro): generate runtime serializer branching for tagged enum codegen
```

---

## Task 12: Remove `config` parameter from codegen entry points

After Tasks 8-11, the `config: &CSharpConfig` parameter should no longer be needed by `into_token_stream` or `build_definition`, since all config decisions are now runtime. Clean up:

**Files:**
- Modify: `crates/csharp-rs-macros/src/codegen/mod.rs` (`into_token_stream`, `build_definition`, `build_export_test`)
- Modify: `crates/csharp-rs-macros/src/lib.rs` (entry point no longer passes config to codegen)

**Step 1: Remove `config` from `into_token_stream`**

```rust
pub fn into_token_stream(self) -> TokenStream {
    // ... no more config parameter ...
}
```

**Step 2: Remove `config` from `build_definition` and `build_export_test`**

The `export_dir` for `build_export_test` needs handling — it was read from `config.export_dir`. Since config is runtime, the export test should use `Config::default()` which already has the default export dir. If a custom `export_to` is specified via attribute, use that. Otherwise, use the C# name to construct the path at runtime from `cfg.export_dir()`.

Update `build_export_test` to generate:

```rust
#[test]
fn export_csharp_mystruct() {
    let cfg = csharp_rs::Config::default();
    let path = format!("{}/{}.cs", cfg.export_dir().display(), "MyStruct");
    csharp_rs::export_to::<MyStruct>(&cfg, path)
        .expect("failed to export C# definition");
}
```

Or if `export_to` attribute is set, use the literal path.

**Step 3: Update entry point**

In `crates/csharp-rs-macros/src/lib.rs`, the config is no longer passed to codegen:

```rust
pub fn derive_csharp(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match types::process_input(&input) {
        Ok(derived) => derived.into_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}
```

**Step 4: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 5: Commit**

```
refactor(macro): remove compile-time config from codegen pipeline
```

---

## Task 13: Remove `CSharpConfig`, `toml` dependency, and compile-time config

Now that no code references `CSharpConfig`, remove it entirely.

**Files:**
- Modify: `crates/csharp-rs-macros/src/config.rs` (remove `CSharpConfig`, `from_manifest_dir`, `from_toml_str`, related tests)
- Modify: `crates/csharp-rs-macros/src/lib.rs` (remove `use config::CSharpConfig` and CARGO_MANIFEST_DIR lines)
- Modify: `crates/csharp-rs-macros/Cargo.toml` (remove `toml.workspace = true`)
- Modify: `Cargo.toml` (remove `toml` from `[workspace.dependencies]`)

**Step 1: Remove `CSharpConfig` from config.rs**

Keep in `config.rs`:
- `CSharpNamespace` struct and validation (still used for `#[csharp(namespace)]` attribute parsing)
- Tests for namespace validation

Remove from `config.rs`:
- `Serializer` enum (lines 12-18) — now in runtime crate
- `CSharpVersion` enum (lines 24-62) — now in runtime crate
- `CSharpConfig` struct (lines 130-220) — replaced by runtime `Config`
- All tests related to `CSharpConfig`, `Serializer`, `CSharpVersion`

The remaining file should contain only `CSharpNamespace`, `validate_namespace`, and namespace tests.

**Step 2: Remove `toml` from Cargo.toml files**

In `crates/csharp-rs-macros/Cargo.toml`, remove:
```toml
toml.workspace = true
```

In root `Cargo.toml`, remove:
```toml
toml = "0.9.12"
```

**Step 3: Clean up entry point**

In `crates/csharp-rs-macros/src/lib.rs`, remove:
- `use config::CSharpConfig;`
- The `CARGO_MANIFEST_DIR` and `CSharpConfig::from_manifest_dir` lines

**Step 4: Fix any remaining references**

Search for `CSharpConfig`, `Serializer` (in macro crate), `CSharpVersion` (in macro crate), `from_manifest_dir`, `from_toml_str`, `toml` in the macro crate. Remove all.

The codegen test helpers (`stj_config()`, `newtonsoft_config()`) that constructed `CSharpConfig` should have been removed in earlier tasks.

**Step 5: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 6: Run full checks**

Run: `cargo fmt --workspace && cargo clippy --workspace`
Expected: ALL PASS

**Step 7: Commit**

```
refactor(macro)!: remove CSharpConfig and toml dependency
```

---

## Task 14: Add multi-config integration tests

Verify that the same type can produce different output with different `Config` values.

**Files:**
- Modify: `crates/csharp-rs/tests/derive_struct.rs` (add multi-config tests)

**Step 1: Write tests**

```rust
#[test]
fn struct_definition_with_newtonsoft() {
    let cfg = Config::default().with_serializer(Serializer::Newtonsoft);
    let def = SimpleStruct::csharp_definition(&cfg);
    assert!(def.contains("using Newtonsoft.Json;"), "should use Newtonsoft:\n{def}");
    assert!(def.contains("[JsonProperty("), "should use JsonProperty:\n{def}");
    assert!(!def.contains("JsonPropertyName"), "should NOT use STJ:\n{def}");
}

#[test]
fn struct_definition_with_system_text_json() {
    let cfg = Config::default().with_serializer(Serializer::SystemTextJson);
    let def = SimpleStruct::csharp_definition(&cfg);
    assert!(def.contains("using System.Text.Json.Serialization;"), "should use STJ:\n{def}");
    assert!(def.contains("[JsonPropertyName("), "should use JsonPropertyName:\n{def}");
    assert!(!def.contains("Newtonsoft"), "should NOT use Newtonsoft:\n{def}");
}

#[test]
fn struct_definition_with_file_scoped_namespace() {
    let cfg = Config::default()
        .with_target(CSharpVersion::CSharp10)
        .with_namespace("My.Custom.Ns");
    let def = SimpleStruct::csharp_definition(&cfg);
    assert!(def.contains("namespace My.Custom.Ns;"), "should use file-scoped ns:\n{def}");
}

#[test]
fn struct_definition_with_block_scoped_namespace() {
    let cfg = Config::default()
        .with_target(CSharpVersion::CSharp9)
        .with_namespace("My.Custom.Ns");
    let def = SimpleStruct::csharp_definition(&cfg);
    assert!(def.contains("namespace My.Custom.Ns\n{"), "should use block-scoped ns:\n{def}");
}

#[test]
fn struct_definition_with_required_modifier() {
    let cfg = Config::default().with_target(CSharpVersion::CSharp11);
    let def = SimpleStruct::csharp_definition(&cfg);
    assert!(def.contains("required"), "C# 11 should use required modifier:\n{def}");
}

#[test]
fn same_type_different_configs() {
    let stj = Config::default();
    let newton = Config::default().with_serializer(Serializer::Newtonsoft);
    let stj_def = SimpleStruct::csharp_definition(&stj);
    let newton_def = SimpleStruct::csharp_definition(&newton);
    assert_ne!(stj_def, newton_def, "different serializers should produce different output");
}
```

**Step 2: Run tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 3: Commit**

```
test(integration): add multi-config tests for runtime serializer and version switching
```

---

## Task 15: Coverage verification and cleanup

**Files:**
- All modified files

**Step 1: Run coverage**

Run: `cargo llvm-cov --workspace --fail-under-lines 98`
Expected: PASS at 98%+

If coverage drops, identify untested paths and add targeted tests.

**Step 2: Run full CI checks**

Run: `cargo fmt --workspace && cargo test --workspace && cargo clippy --workspace`
Expected: ALL PASS with no warnings.

**Step 3: Final commit (if any cleanup needed)**

```
chore(workspace): coverage and lint cleanup for runtime config migration
```

---

## Execution Notes

### Task ordering dependencies

```
Task 1 → Task 2 → Task 3 (types build on each other)
Task 3 → Task 4 (Config must exist before trait uses it)
Task 4 → Task 5 (trait signature must change before codegen)
Task 5 → Task 6 (codegen must match before integration tests compile)
Task 5 → Task 7 (codegen must match before codegen tests pass)
Task 5 → Task 8 (codegen signature established before namespace refactor)
Task 8 → Task 9 → Task 10 → Task 11 (serializer runtime branching, building complexity)
Task 11 → Task 12 (all runtime branching done before removing config param)
Task 12 → Task 13 (config param removed before deleting CSharpConfig)
Task 13 → Task 14 (clean slate for multi-config tests)
Task 14 → Task 15 (all features done before coverage check)
```

### Verification command at any point

```bash
cargo fmt --workspace && cargo test --workspace && cargo clippy --workspace
```

### Key risk: tagged enum codegen complexity

Task 11 is the largest single task (~3000 lines of codegen). The 4 tagging modes each generate different C# patterns, and the native polymorphism path (Internal + STJ + C# 11+) is particularly involved. Consider splitting Task 11 into sub-tasks per tagging mode if it proves unwieldy during execution.

### Key insight: `type_expr` token streams

The `CSharpField.type_expr` and `TaggedVariantData::Newtype.type_expr` fields store token fragments like `<String as csharp_rs::CSharp>::csharp_name(cfg)`. These are evaluated in the generated code's `csharp_definition(cfg)` method body, so `cfg` is in scope. This is why Task 5 must update these expressions to pass `cfg`.
