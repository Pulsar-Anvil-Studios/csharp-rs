## [0.1.2] - 2026-03-18

### 🐛 Bug Fixes

- *(hooks)* Use cross-platform regex for coverage exclusion
- *(macros)* Suppress nullable suffix when csharp type override is present

### ⚙️ Miscellaneous Tasks

- *(release)* V0.1.1 [skip ci]
- *(hooks)* Add pre-commit and pre-push git hooks
## [0.1.1] - 2026-03-15

### 🚀 Features

- *(workspace)* Scaffold csharp-rs derive macro workspace
- *(macro)* Implement CSharp derive for named structs with strong config types
- *(macro)* Add field-level serde attribute support (#2)
- *(macro)* Add simple enum support with unit variants (#3)
- *(macro)* Add tagged enum support with all 4 serde modes (#4)
- *(macro)* Add serde flatten support for structs and tagged enums (#5)
- *(csharp-rs)* [**breaking**] Add runtime Config and update CSharp trait with cfg parameter
- *(macro)* [**breaking**] Update codegen for runtime Config and namespace_override
- *(codegen)* [**breaking**] Complete runtime branching for all codegen modules
- *(attr)* Parse serde default field attribute
- *(attr)* Parse serde rename_all_fields container attribute
- *(types)* Serde default marks fields as optional/nullable
- *(attr)* Parse csharp type override field attribute
- *(types)* Apply rename_all_fields to variant struct fields
- *(types)* Wire csharp type override into codegen
- *(attr)* Parse serde transparent container attribute
- *(types)* Add newtype struct handler with Value property
- *(codegen)* Add transparent converter for newtype records
- *(config)* Add CSharpVersion::Unity with helper methods
- *(codegen)* Support Unity class output in record codegen
- *(types)* Add external type impls for uuid, chrono, and serde_json
- *(config)* Add Config::from_env() for environment-based configuration
- *(codegen)* Add generic type parameter support

### 🐛 Bug Fixes

- *(test)* Suppress dotnet first-time setup race condition in compilation tests
- *(codegen)* Always include using System in generated C# files

### 🚜 Refactor

- *(macro)* [**breaking**] Remove compile-time config and toml dependency
- *(types)* Add transparent field to DerivedCSharp IR
- *(codegen)* Use Config::from_env() in generated export tests

### 📚 Documentation

- *(plans)* Add step 6 runtime config design document

### 🧪 Testing

- *(integration)* Add multi-config tests for runtime serializer and version switching
- *(integration)* Add newtype and transparent newtype tests
- *(compilation)* Add C# compilation verification tests for all features and versions
- *(generics)* Add integration and C# compilation tests for generics
- *(roundtrip)* Add cross-language JSON round-trip E2E tests

### ⚙️ Miscellaneous Tasks

- *(github)* Add coverage reporting with 98% threshold (#1)
- *(doc)* Remove opencode doc
- *(git)* Add .opencode to gitignore
- *(github)* Add C# compilation verification job to CI workflow
- *(github)* Update workflow actions to latest major versions
- *(readme)* Add readme and licence
- *(cargo)* Prepare Cargo.toml metadata for crates.io publishing
- *(github)* Add tag-triggered release workflow for crates.io publishing
- *(fmt)* Apply rustfmt formatting to generics and test files
- *(release)* V0.1.0 [skip ci]
- *(release)* Trigger v0.1.1 release
