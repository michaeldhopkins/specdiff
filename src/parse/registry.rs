use serde::Deserialize;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
pub struct TomlFile {
    pub framework: Vec<FrameworkDef>,
}

#[derive(Debug, Deserialize)]
pub struct FrameworkDef {
    pub name: String,
    pub language: String,
    pub tree_sitter_crate: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub additional_languages: Vec<String>,
    #[serde(default)]
    pub additional_crates: Vec<String>,
    #[serde(default)]
    pub files: Option<FileDef>,
    #[serde(default)]
    pub group: Vec<GroupDef>,
    #[serde(default)]
    pub spec: Vec<SpecDef>,
    #[serde(default)]
    pub marker: Vec<MarkerDef>,
    #[serde(default)]
    pub normalization: Option<NormalizationDef>,
    #[serde(default)]
    pub path_grouping: Option<PathGroupingDef>,
    #[serde(default)]
    pub shared: Option<SharedDef>,
    #[serde(default)]
    pub parameterized: Vec<ParameterizedDef>,
    #[serde(default)]
    pub property_based: Vec<PropertyBasedDef>,
    #[serde(default)]
    pub nested_discovery: Vec<NestedDiscoveryDef>,
    #[serde(default)]
    pub table_driven: Option<TableDrivenDef>,
    #[serde(default)]
    pub ast_kinds: Option<AstKindsDef>,
    #[serde(default)]
    pub inheritance: Option<InheritanceDef>,
    #[serde(default)]
    pub handler: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AstKindsDef {
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub body_field: Option<String>,
    #[serde(default)]
    pub body_child: Option<String>,
    #[serde(default)]
    pub name_field: Option<String>,
    #[serde(default)]
    pub name_child: Option<String>,
    #[serde(default)]
    pub name_child_alt: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InheritanceDef {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub type_node_kinds: Vec<String>,
    #[serde(default)]
    pub inert_containers: Vec<String>,
    #[serde(default)]
    pub scan_globs: Vec<String>,
    #[serde(default)]
    pub ref_detection: Vec<RefDetectionDef>,
}

#[derive(Debug, Deserialize)]
pub struct RefDetectionDef {
    pub strategy: String,
    #[serde(default)]
    pub container_kinds: Vec<String>,
    #[serde(default)]
    pub method_names: Vec<String>,
    #[serde(default)]
    pub arg_kinds: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileDef {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub include_globs: Vec<String>,
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    #[serde(default)]
    pub require_ast_confirmation: bool,
}

#[derive(Debug, Deserialize)]
pub struct GroupDef {
    pub ast_type: String,
    #[serde(default)]
    pub method_names: Vec<String>,
    #[serde(default)]
    pub name_source: String,
    #[serde(default)]
    pub name_source_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SpecDef {
    pub ast_type: String,
    #[serde(default)]
    pub method_names: Vec<String>,
    #[serde(default)]
    pub name_source: String,
    #[serde(default)]
    pub name_source_type: Option<String>,
    #[serde(default)]
    pub allow_anonymous: bool,
}

#[derive(Debug, Deserialize)]
pub struct MarkerDef {
    pub marker_type: String,
    #[serde(default)]
    pub marker_name: Option<String>,
    #[serde(default)]
    pub marker_argument: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub applies_to: String,
    #[serde(default)]
    pub creates: String,
    #[serde(default)]
    pub required_param_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct NormalizationDef {
    #[serde(default)]
    pub strip_prefixes: Vec<String>,
    #[serde(default)]
    pub strip_suffixes: Vec<String>,
    #[serde(default)]
    pub underscore_to_space: bool,
    #[serde(default)]
    pub strip_camel_test_prefix: bool,
    #[serde(default)]
    pub raw: bool,
}

#[derive(Debug, Deserialize)]
pub struct PathGroupingDef {
    #[serde(default)]
    pub strip_prefixes: Vec<String>,
    #[serde(default)]
    pub strip_suffixes: Vec<String>,
    #[serde(default)]
    pub separator: String,
    #[serde(default)]
    pub strip_extension: bool,
}

#[derive(Debug, Deserialize)]
pub struct SharedDef {
    #[serde(default)]
    pub definition_globs: Vec<String>,
    #[serde(default)]
    pub scan_spec_files_for_definitions: bool,
    #[serde(default)]
    pub definition: Vec<SharedDefinitionDef>,
    #[serde(default)]
    pub inclusion: Vec<SharedInclusionDef>,
}

#[derive(Debug, Deserialize)]
pub struct SharedDefinitionDef {
    pub ast_type: String,
    #[serde(default)]
    pub method_names: Vec<String>,
    #[serde(default)]
    pub name_source: String,
    #[serde(default)]
    pub name_source_type: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub detection_strategy: Option<String>,
    #[serde(default)]
    pub handler: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SharedInclusionDef {
    pub ast_type: String,
    #[serde(default)]
    pub method_names: Vec<String>,
    #[serde(default)]
    pub name_source: String,
    #[serde(default)]
    pub name_source_type: Option<String>,
    #[serde(default)]
    pub nesting: Option<String>,
    #[serde(default)]
    pub nested_name_template: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ParameterizedDef {
    pub detection: String,
    #[serde(default)]
    pub decorator_name: Option<String>,
    #[serde(default)]
    pub attribute_name: Option<String>,
    #[serde(default)]
    pub method_names: Vec<String>,
    #[serde(default)]
    pub case_count_source: Option<String>,
    #[serde(default)]
    pub case_count_type: Option<String>,
    #[serde(default)]
    pub label_source: Option<String>,
    #[serde(default)]
    pub label_fallback: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PropertyBasedDef {
    pub detection: String,
    #[serde(default)]
    pub macro_name: Option<String>,
    #[serde(default)]
    pub decorator_name: Option<String>,
    #[serde(default)]
    pub decorator_module: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NestedDiscoveryDef {
    #[serde(default)]
    pub parent: String,
    pub ast_type: String,
    #[serde(default)]
    pub receiver: Option<String>,
    #[serde(default)]
    pub method_name: Option<String>,
    #[serde(default)]
    pub name_source: String,
    #[serde(default)]
    pub name_source_type: Option<String>,
    #[serde(default)]
    pub creates: String,
}

#[derive(Debug, Deserialize)]
pub struct TableDrivenDef {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub slice_pattern: Option<String>,
    #[serde(default)]
    pub loop_pattern: Option<String>,
    #[serde(default)]
    pub run_call: Option<String>,
}

include!(concat!(env!("OUT_DIR"), "/framework_includes.rs"));

static FRAMEWORKS: LazyLock<Vec<FrameworkDef>> = LazyLock::new(|| {
    let mut frameworks = Vec::new();
    for toml_str in framework_toml_strings() {
        match toml::from_str::<TomlFile>(toml_str) {
            Ok(file) => frameworks.extend(file.framework),
            Err(e) => eprintln!("failed to parse framework TOML: {e}"),
        }
    }
    frameworks
});

pub fn all_frameworks() -> &'static [FrameworkDef] {
    &FRAMEWORKS
}

pub fn frameworks_for_file(path: &Path) -> Vec<&'static FrameworkDef> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"));
    let path_str = path.to_string_lossy();

    all_frameworks()
        .iter()
        .filter(|fw| {
            let Some(files) = &fw.files else {
                return false;
            };

            let ext_matches = extension
                .as_ref()
                .is_some_and(|ext| files.extensions.contains(ext));
            if !ext_matches {
                return false;
            }

            if files.include_globs.is_empty() {
                return true;
            }

            let included = files.include_globs.iter().any(|g| {
                glob::Pattern::new(g)
                    .is_ok_and(|pat| pat.matches(&path_str))
            });
            if !included {
                return false;
            }

            let excluded = files.exclude_globs.iter().any(|g| {
                glob::Pattern::new(g)
                    .is_ok_and(|pat| pat.matches(&path_str))
            });

            !excluded
        })
        .collect()
}

pub fn normalize_file_path(path: &str, framework: &FrameworkDef) -> String {
    let pg = match &framework.path_grouping {
        Some(pg) => pg,
        None => return path.to_string(),
    };

    let mut result = path.to_string();

    for prefix in &pg.strip_prefixes {
        if let Some(stripped) = result.strip_prefix(prefix.as_str()) {
            result = stripped.to_string();
            break;
        }
    }

    if pg.strip_extension && let Some(dot) = result.rfind('.') {
        result.truncate(dot);
    }

    loop {
        let mut changed = false;
        for suffix in &pg.strip_suffixes {
            if let Some(stripped) = result.strip_suffix(suffix.as_str()) {
                result = stripped.to_string();
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }

    if !pg.separator.is_empty() {
        result = result.replace('/', &pg.separator);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_framework_tomls_deserialize() {
        let frameworks = all_frameworks();
        assert!(!frameworks.is_empty(), "no frameworks loaded");
        for fw in frameworks {
            assert!(!fw.name.is_empty(), "framework has empty name");
            assert!(!fw.language.is_empty(), "framework {} has empty language", fw.name);
        }
    }

    #[test]
    fn rspec_framework_loads() {
        let fw = all_frameworks().iter().find(|f| f.name == "rspec");
        assert!(fw.is_some(), "rspec framework not found");
        let fw = fw.expect("checked above");
        assert_eq!(fw.language, "ruby");
        assert!(!fw.group.is_empty());
        assert!(!fw.spec.is_empty());
        assert!(fw.shared.is_some());
    }

    #[test]
    fn rust_builtin_framework_loads() {
        let fw = all_frameworks().iter().find(|f| f.name == "rust_builtin");
        assert!(fw.is_some(), "rust_builtin framework not found");
        let fw = fw.expect("checked above");
        assert_eq!(fw.language, "rust");
        assert!(!fw.marker.is_empty());
    }

    #[test]
    fn frameworks_for_ruby_file() {
        let matches = frameworks_for_file(Path::new("spec/models/user_spec.rb"));
        assert!(matches.iter().any(|f| f.name == "rspec"));
    }

    #[test]
    fn frameworks_for_rust_file() {
        let matches = frameworks_for_file(Path::new("src/lib.rs"));
        assert!(matches.iter().any(|f| f.name == "rust_builtin"));
    }

    #[test]
    fn ruby_file_outside_spec_dir_does_not_match_rspec() {
        let matches = frameworks_for_file(Path::new("lib/models/user.rb"));
        assert!(!matches.iter().any(|f| f.name == "rspec"), "lib/ .rb should not match rspec");
    }

    #[test]
    fn minitest_file_matches_test_dir() {
        let matches = frameworks_for_file(Path::new("test/models/user_test.rb"));
        assert!(matches.iter().any(|f| f.name == "minitest"));
    }

    #[test]
    fn ruby_file_outside_test_dir_does_not_match_minitest() {
        let matches = frameworks_for_file(Path::new("app/models/user.rb"));
        assert!(!matches.iter().any(|f| f.name == "minitest"), "app/ .rb should not match minitest");
    }

    #[test]
    fn excluded_files_do_not_match() {
        let matches = frameworks_for_file(Path::new("spec/spec_helper.rb"));
        assert!(!matches.iter().any(|f| f.name == "rspec"), "spec_helper.rb should be excluded");
    }

    #[test]
    fn normalize_rspec_path() {
        let rspec = all_frameworks().iter().find(|f| f.name == "rspec").expect("rspec");
        assert_eq!(normalize_file_path("spec/models/user_spec.rb", rspec), "models::user");
    }

    #[test]
    fn normalize_rust_path() {
        let rust = all_frameworks().iter().find(|f| f.name == "rust_builtin").expect("rust");
        assert_eq!(normalize_file_path("src/lib.rs", rust), "lib");
        assert_eq!(normalize_file_path("tests/integration.rs", rust), "integration");
    }

    #[test]
    fn normalize_pytest_path() {
        let pytest = all_frameworks().iter().find(|f| f.name == "pytest").expect("pytest");
        assert_eq!(normalize_file_path("tests/test_user.py", pytest), "test_user");
    }

    #[test]
    fn normalize_go_path() {
        let go = all_frameworks().iter().find(|f| f.name == "go_testing").expect("go");
        assert_eq!(normalize_file_path("user_test.go", go), "user");
    }

    #[test]
    fn normalize_jest_path() {
        let jest = all_frameworks().iter().find(|f| f.name == "jest").expect("jest");
        assert_eq!(normalize_file_path("__tests__/user.test.js", jest), "user");
    }

    #[test]
    fn normalize_minitest_path() {
        let minitest = all_frameworks().iter().find(|f| f.name == "minitest").expect("minitest");
        assert_eq!(normalize_file_path("test/models/user_test.rb", minitest), "models::user");
    }

    #[test]
    fn normalize_double_suffixed_jest_path() {
        let jest = all_frameworks().iter().find(|f| f.name == "jest").expect("jest");
        assert_eq!(normalize_file_path("__tests__/user.test.spec.js", jest), "user");
    }
}
