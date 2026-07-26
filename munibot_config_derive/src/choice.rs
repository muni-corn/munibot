use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Result};

use crate::attrs::parse_variant_attrs;

/// Expands `#[derive(ConfigChoice)]`.
///
/// Only applies to enums whose variants all carry no data. Generates
/// `impl ConfigChoice`, `impl ConfigField` (via the `choice_field_kind`/
/// `choice_to_value`/`choice_from_value` helpers in `munibot_config`, so
/// there's no risk of drifting from how `Configurable` fields expect a
/// `ConfigField` impl to behave), and -- deliberately, rather than relying
/// on a separately-derived `#[derive(Serialize, Deserialize)]` that could
/// use different variant names -- `Serialize`/`Deserialize` delegating to
/// `choice_id`/`from_choice_id`. See docs/notes for why.
pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let ident = &input.ident;

    let Data::Enum(data_enum) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input,
            "ConfigChoice can only be derived for an enum",
        ));
    };

    let mut ids = Vec::new();
    let mut labels = Vec::new();
    let mut variant_idents = Vec::new();
    let mut seen_ids = HashSet::new();

    for variant in &data_enum.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "ConfigChoice variants must not carry data",
            ));
        }

        let variant_attrs = parse_variant_attrs(&variant.attrs)?;
        let default_id = to_snake_case(&variant.ident.to_string());
        let id = variant_attrs
            .id
            .as_ref()
            .map(|lit| lit.value())
            .unwrap_or_else(|| default_id.clone());
        let label = variant_attrs
            .label
            .as_ref()
            .map(|lit| lit.value())
            .unwrap_or_else(|| humanize(&default_id));

        if !seen_ids.insert(id.clone()) {
            return Err(syn::Error::new_spanned(
                variant,
                format!("duplicate ConfigChoice id `{id}`"),
            ));
        }

        ids.push(id);
        labels.push(label);
        variant_idents.push(&variant.ident);
    }

    Ok(quote! {
        impl ::munibot_config::ConfigChoice for #ident {
            fn choice_options() -> ::std::vec::Vec<::munibot_config::ChoiceOption> {
                ::std::vec![
                    #(::munibot_config::ChoiceOption { id: #ids.to_owned(), label: #labels.to_owned() }),*
                ]
            }

            fn choice_id(&self) -> ::std::string::String {
                match self {
                    #(Self::#variant_idents => #ids.to_owned(),)*
                }
            }

            fn from_choice_id(id: &str) -> ::std::option::Option<Self> {
                match id {
                    #(#ids => ::std::option::Option::Some(Self::#variant_idents),)*
                    _ => ::std::option::Option::None,
                }
            }
        }

        impl ::munibot_config::ConfigField for #ident {
            fn field_kind() -> ::munibot_config::FieldKind {
                ::munibot_config::choice_field_kind::<Self>()
            }

            fn to_value(&self) -> ::munibot_config::ConfigValue {
                ::munibot_config::choice_to_value(self)
            }

            fn from_value(
                value: &::munibot_config::ConfigValue,
            ) -> ::std::result::Result<Self, ::munibot_config::ValueError> {
                ::munibot_config::choice_from_value(value)
            }
        }

        impl ::serde::Serialize for #ident {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&::munibot_config::ConfigChoice::choice_id(self))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for #ident {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> ::std::result::Result<Self, D::Error> {
                let id = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                <Self as ::munibot_config::ConfigChoice>::from_choice_id(&id)
                    .ok_or_else(|| ::serde::de::Error::custom(::std::format!("unknown choice id `{id}`")))
            }
        }
    })
}

/// `AfterSilence` -> `"after_silence"`. The fallback id when a variant has
/// no `#[config(id = ...)]`.
fn to_snake_case(variant_name: &str) -> String {
    let mut out = String::new();
    for (i, c) in variant_name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `after_silence` -> `"after silence"`. The fallback label when a variant
/// has no `#[config(label = ...)]`.
fn humanize(id: &str) -> String {
    id.replace('_', " ")
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::{expand, to_snake_case};

    #[test]
    fn test_to_snake_case_converts_pascal_case() {
        assert_eq!(to_snake_case("AfterSilence"), "after_silence");
        assert_eq!(to_snake_case("Always"), "always");
    }

    #[test]
    fn test_expand_rejects_a_struct() {
        let input = parse_quote! {
            struct X { a: bool }
        };
        assert!(expand(input).is_err());
    }

    #[test]
    fn test_expand_rejects_a_variant_with_fields() {
        let input = parse_quote! {
            enum X {
                A(bool),
            }
        };
        assert!(expand(input).is_err());
    }

    #[test]
    fn test_expand_rejects_duplicate_ids() {
        let input = parse_quote! {
            enum X {
                #[config(id = "same")]
                A,
                #[config(id = "same")]
                B,
            }
        };
        assert!(expand(input).is_err());
    }

    #[test]
    fn test_expand_accepts_a_fieldless_enum() {
        let input = parse_quote! {
            enum CleanupMode {
                Always,
                AfterSilence,
            }
        };
        assert!(expand(input).is_ok());
    }
}
