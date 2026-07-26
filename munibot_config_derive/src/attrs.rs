// not wired into the actual derives yet -- that lands with the codegen
// commits that follow this one. remove once config_fields/descriptor
// generation (commit 17) calls these.
#![allow(dead_code)]

use syn::{Attribute, Expr, Ident, LitStr, Result};

/// Parsed `#[config(...)]` attributes from a `Configurable` container
/// (struct or enum).
///
/// `id`/`label`/`scope` are only present for a provider's root type -- see
/// `Configurable::descriptor` in munibot_config. A nested type used only as
/// `Section`/`Collection` content has none of these.
#[derive(Debug, Default)]
pub struct ContainerAttrs {
    pub id: Option<LitStr>,
    pub label: Option<LitStr>,
    pub description: Option<LitStr>,
    pub icon: Option<LitStr>,
    /// A bare `ScopeKind` variant name, e.g. `scope = DiscordGuild`. Only a
    /// single scope is supported in v1 -- see
    /// docs/notes/config-schema-implementation-notes.md.
    pub scope: Option<Ident>,
}

/// Parsed `#[config(...)]` attributes from a single field of a
/// `Configurable` struct.
///
/// `min`/`max`/`max_len` are captured as raw expressions rather than a
/// specific literal type: what they mean (a plain integer, or a count of
/// characters) depends on the field's own type, and splicing an untyped
/// expression into the generated code lets ordinary Rust type inference
/// resolve that rather than this crate needing to.
#[derive(Debug, Default)]
pub struct FieldAttrs {
    pub label: Option<LitStr>,
    pub description: Option<LitStr>,
    pub min: Option<Expr>,
    pub max: Option<Expr>,
    pub max_len: Option<Expr>,
    pub placeholder: Option<LitStr>,
    pub multiline: bool,
    /// Only meaningful on a `Collection` (`BTreeMap<K, V>`) field.
    pub add_label: Option<LitStr>,
    pub required: bool,
    /// Omits this field from the generated schema entirely.
    pub skip: bool,
}

/// Parses every `#[config(...)]` attribute on a container into one
/// `ContainerAttrs`, later keys overwriting earlier ones of the same name.
pub fn parse_container_attrs(attrs: &[Attribute]) -> Result<ContainerAttrs> {
    let mut out = ContainerAttrs::default();

    for attr in config_attrs(attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                out.id = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("label") {
                out.label = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("description") {
                out.description = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("icon") {
                out.icon = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("scope") {
                out.scope = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("unrecognized container #[config(...)] key"));
            }
            Ok(())
        })?;
    }

    Ok(out)
}

/// Parses every `#[config(...)]` attribute on a field into one
/// `FieldAttrs`, later keys overwriting earlier ones of the same name.
pub fn parse_field_attrs(attrs: &[Attribute]) -> Result<FieldAttrs> {
    let mut out = FieldAttrs::default();

    for attr in config_attrs(attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("label") {
                out.label = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("description") {
                out.description = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("min") {
                out.min = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("max") {
                out.max = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("max_len") {
                out.max_len = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("placeholder") {
                out.placeholder = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("multiline") {
                out.multiline = true;
            } else if meta.path.is_ident("add_label") {
                out.add_label = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("required") {
                out.required = true;
            } else if meta.path.is_ident("skip") {
                out.skip = true;
            } else {
                return Err(meta.error("unrecognized field #[config(...)] key"));
            }
            Ok(())
        })?;
    }

    Ok(out)
}

fn config_attrs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|attr| attr.path().is_ident("config"))
}

#[cfg(test)]
mod tests {
    use syn::{DeriveInput, parse_quote};

    use super::{parse_container_attrs, parse_field_attrs};

    #[test]
    fn test_parse_container_attrs_reads_every_key() {
        let input: DeriveInput = parse_quote! {
            #[config(id = "logging", label = "server logging", description = "posts events", icon = "ph-bell", scope = DiscordGuild)]
            struct LoggingConfig;
        };
        let attrs = parse_container_attrs(&input.attrs).expect("should parse");

        assert_eq!(attrs.id.unwrap().value(), "logging");
        assert_eq!(attrs.label.unwrap().value(), "server logging");
        assert_eq!(attrs.description.unwrap().value(), "posts events");
        assert_eq!(attrs.icon.unwrap().value(), "ph-bell");
        assert_eq!(attrs.scope.unwrap(), "DiscordGuild");
    }

    #[test]
    fn test_parse_container_attrs_defaults_to_all_none_without_id() {
        let input: DeriveInput = parse_quote! {
            struct LoggingSettings;
        };
        let attrs = parse_container_attrs(&input.attrs).expect("should parse");
        assert!(attrs.id.is_none());
        assert!(attrs.scope.is_none());
    }

    #[test]
    fn test_parse_container_attrs_rejects_an_unknown_key() {
        let input: DeriveInput = parse_quote! {
            #[config(bogus = "1")]
            struct X;
        };
        assert!(parse_container_attrs(&input.attrs).is_err());
    }

    #[test]
    fn test_parse_field_attrs_reads_value_and_flag_keys() {
        let input: DeriveInput = parse_quote! {
            struct X {
                #[config(label = "message lifetime", min = 3600, max = 604800, required)]
                lifetime: u64,
            }
        };
        let syn::Data::Struct(data) = &input.data else {
            panic!("expected a struct")
        };
        let field = data.fields.iter().next().expect("one field");
        let attrs = parse_field_attrs(&field.attrs).expect("should parse");

        assert_eq!(attrs.label.unwrap().value(), "message lifetime");
        assert!(attrs.min.is_some());
        assert!(attrs.max.is_some());
        assert!(attrs.required);
        assert!(!attrs.skip);
    }

    #[test]
    fn test_parse_field_attrs_reads_skip_and_multiline_flags() {
        let input: DeriveInput = parse_quote! {
            struct X {
                #[config(skip)]
                internal: u64,
                #[config(multiline)]
                notes: String,
            }
        };
        let syn::Data::Struct(data) = &input.data else {
            panic!("expected a struct")
        };
        let mut fields = data.fields.iter();
        let internal = parse_field_attrs(&fields.next().unwrap().attrs).expect("should parse");
        let notes = parse_field_attrs(&fields.next().unwrap().attrs).expect("should parse");

        assert!(internal.skip);
        assert!(notes.multiline);
    }

    #[test]
    fn test_parse_field_attrs_ignores_attributes_from_other_macros() {
        let input: DeriveInput = parse_quote! {
            struct X {
                #[serde(default)]
                #[config(label = "amount")]
                amount: i64,
            }
        };
        let syn::Data::Struct(data) = &input.data else {
            panic!("expected a struct")
        };
        let field = data.fields.iter().next().expect("one field");
        let attrs = parse_field_attrs(&field.attrs).expect("should parse");
        assert_eq!(attrs.label.unwrap().value(), "amount");
    }

    #[test]
    fn test_parse_field_attrs_rejects_an_unknown_key() {
        let input: DeriveInput = parse_quote! {
            struct X {
                #[config(bogus = "1")]
                field: i64,
            }
        };
        let syn::Data::Struct(data) = &input.data else {
            panic!("expected a struct")
        };
        let field = data.fields.iter().next().expect("one field");
        assert!(parse_field_attrs(&field.attrs).is_err());
    }
}
