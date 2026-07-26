use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Result};

use crate::{
    attrs::{ContainerAttrs, FieldAttrs, parse_container_attrs, parse_field_attrs},
    shape::{FieldShape, classify_field_shape},
};

/// Expands `#[derive(Configurable)]`.
///
/// v1 only supports structs with named fields -- see
/// docs/notes/config-schema-implementation-notes.md for the field shapes
/// this recognizes and the ones it deliberately doesn't yet.
pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let ident = &input.ident;
    let container_attrs = parse_container_attrs(&input.attrs)?;
    let descriptor_impl = generate_descriptor(&input, &container_attrs)?;

    let Data::Struct(data_struct) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input,
            "Configurable can only be derived for a struct",
        ));
    };
    let Fields::Named(fields_named) = &data_struct.fields else {
        return Err(syn::Error::new_spanned(
            &data_struct.fields,
            "Configurable requires named fields",
        ));
    };

    let mut field_nodes = Vec::new();
    for field in &fields_named.named {
        let field_attrs = parse_field_attrs(&field.attrs)?;
        if field_attrs.skip {
            continue;
        }

        // a named field always has an ident; only tuple/unit struct fields
        // don't, and those were already rejected above.
        let field_ident = field.ident.as_ref().expect("named field has an ident");

        let node = match classify_field_shape(&field.ty) {
            FieldShape::Leaf => generate_field_node(field_ident, &field.ty, &field_attrs),
            FieldShape::Toggled { settings_ty } => {
                generate_section_node(field_ident, settings_ty, &field_attrs)
            }
            FieldShape::Collection { key_ty, value_ty } => {
                generate_collection_node(field_ident, key_ty, value_ty, &field_attrs)
            }
        };
        field_nodes.push(node);
    }

    Ok(quote! {
        impl ::munibot_config::Configurable for #ident {
            fn config_fields() -> Vec<::munibot_config::ConfigNode> {
                vec![ #(#field_nodes),* ]
            }

            #descriptor_impl
        }
    })
}

/// Builds `fn descriptor() -> ProviderDescriptor { ... }` if the container
/// declared `id`/`label`/`scope`, or `None` if it declared none of them
/// (a nested type, relying on the trait's default `descriptor()`).
///
/// Errors if only *some* of `id`/`label`/`scope` are present: that's
/// ambiguous enough to be worth rejecting rather than silently treating the
/// container as either "a root" or "not a root".
fn generate_descriptor(input: &DeriveInput, attrs: &ContainerAttrs) -> Result<Option<TokenStream>> {
    let present = [
        attrs.id.is_some(),
        attrs.label.is_some(),
        attrs.scope.is_some(),
    ];

    if present == [false, false, false] {
        return Ok(None);
    }
    if present != [true, true, true] {
        return Err(syn::Error::new_spanned(
            input,
            "a provider's root #[derive(Configurable)] type needs id, label, and scope together \
             in #[config(...)] -- a nested type (used only as Section/Collection content) needs \
             none of them",
        ));
    }

    let id = attrs.id.as_ref().unwrap();
    let label = attrs.label.as_ref().unwrap();
    let scope = attrs.scope.as_ref().unwrap();
    let description = opt_lit_expr(&attrs.description);
    let icon = opt_lit_expr(&attrs.icon);

    Ok(Some(quote! {
        fn descriptor() -> ::munibot_config::ProviderDescriptor {
            ::munibot_config::ProviderDescriptor {
                id: #id.to_owned(),
                label: #label.to_owned(),
                description: #description.map(|s: &str| s.to_owned()),
                icon: #icon.map(|s: &str| s.to_owned()),
                scopes: vec![::munibot_config::ScopeKind::#scope],
            }
        }
    }))
}

/// Builds an `Option<&str>` expression from a parsed `Option<LitStr>`
/// attribute value.
///
/// Not just `#lit_str_option` spliced directly: quote's `ToTokens` for
/// `Option<T>` emits the literal's own tokens when `Some`, or *no tokens at
/// all* when `None` -- not the Rust tokens `Some(...)`/`None`. Splicing
/// that as an expression works when the value is a whole standalone
/// expression, but produces invalid syntax where `None` needs to still be
/// a valid `Option` value (e.g. as an argument to `.map(...)`).
fn opt_lit_expr(lit: &Option<syn::LitStr>) -> TokenStream {
    // explicitly typed as Option<&str>: an untyped `None` here is only ever
    // used as an argument to a generic `impl Into<String>` parameter (see
    // Field::description), which isn't enough for rustc to infer what
    // `None`'s type parameter should be, and fails with E0282.
    match lit {
        Some(s) => quote! { ::std::option::Option::<&str>::Some(#s) },
        None => quote! { ::std::option::Option::<&str>::None },
    }
}

/// Builds a `ConfigNode::Field(...)` for a leaf field.
///
/// The field's default is always `Self::default()`'s own value for this
/// field, converted through `ConfigField::to_value` -- never a separate
/// `#[config(default = ...)]` attribute, so there is exactly one place a
/// provider author states a default (see docs/notes).
fn generate_field_node(
    field_ident: &Ident,
    field_ty: &syn::Type,
    attrs: &FieldAttrs,
) -> TokenStream {
    let field_name = field_ident.to_string();
    let label = attrs
        .label
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_else(|| humanize(&field_name));
    let description = opt_lit_expr(&attrs.description);
    let required = attrs.required;

    quote! {
        ::munibot_config::ConfigNode::Field({
            #[allow(unused_mut)]
            let mut field = ::munibot_config::Field::new(
                #field_name,
                #label,
                <#field_ty as ::munibot_config::ConfigField>::field_kind(),
            )
            .default(<#field_ty as ::munibot_config::ConfigField>::to_value(&Self::default().#field_ident));

            if let ::std::option::Option::Some(description) = #description {
                field = field.description(description);
            }
            if #required {
                field = field.required();
            }

            field
        })
    }
}

/// Builds a `ConfigNode::Section` for a `Toggled<T>` field.
///
/// The section's own `label`/`description` come from the field's
/// `#[config(...)]` attributes, same as a leaf field. The toggle itself
/// always has the fixed label "enabled" -- the section's own label already
/// gives context (e.g. a "server logging" card with an "enabled" switch
/// inside it), so there's no separate attribute for it in v1. This departs
/// from docs/plans/gui-configuration.md section 6.3's example, whose single
/// `label` attribute on the field couldn't unambiguously mean both "the
/// section's heading" and "the toggle's own label" at once.
fn generate_section_node(
    field_ident: &Ident,
    settings_ty: &syn::Type,
    attrs: &FieldAttrs,
) -> TokenStream {
    let field_name = field_ident.to_string();
    let label = attrs
        .label
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_else(|| humanize(&field_name));
    let description = opt_lit_expr(&attrs.description);

    quote! {
        ::munibot_config::ConfigNode::Section {
            id: #field_name.to_owned(),
            label: #label.to_owned(),
            description: #description.map(|s: &str| s.to_owned()),
            toggle: ::std::option::Option::Some(
                ::munibot_config::Field::new("enabled", "enabled", ::munibot_config::FieldKind::Bool)
                    .default(<bool as ::munibot_config::ConfigField>::to_value(
                        &Self::default().#field_ident.enabled,
                    )),
            ),
            children: <#settings_ty as ::munibot_config::Configurable>::config_fields(),
        }
    }
}

/// Builds a `ConfigNode::Collection` for a `BTreeMap<K, V>` field.
///
/// Unlike `Section`, `Collection` has no `description` field in
/// `munibot_config` -- a `#[config(description = ...)]` on one of these
/// fields is silently unused rather than rejected, matching how
/// `required`/`min`/`max`/etc are also just ignored on shapes they don't
/// apply to in v1 (see docs/notes for the broader "no per-shape attribute
/// validation yet" scope cut).
fn generate_collection_node(
    field_ident: &Ident,
    key_ty: &syn::Type,
    value_ty: &syn::Type,
    attrs: &FieldAttrs,
) -> TokenStream {
    let field_name = field_ident.to_string();
    let label = attrs
        .label
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_else(|| humanize(&field_name));
    let add_label = attrs
        .add_label
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_else(|| format!("add {label}"));

    quote! {
        ::munibot_config::ConfigNode::Collection {
            id: #field_name.to_owned(),
            label: #label.to_owned(),
            key: <#key_ty as ::munibot_config::ConfigKey>::key_kind(),
            add_label: #add_label.to_owned(),
            item: <#value_ty as ::munibot_config::Configurable>::config_fields(),
        }
    }
}

/// `log_channel` -> `"log channel"`. The fallback label when a field has no
/// `#[config(label = ...)]`.
fn humanize(field_name: &str) -> String {
    field_name.replace('_', " ")
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::{expand, humanize};

    #[test]
    fn test_humanize_replaces_underscores_with_spaces() {
        assert_eq!(humanize("log_channel"), "log channel");
        assert_eq!(humanize("channel"), "channel");
    }

    #[test]
    fn test_expand_rejects_a_non_struct() {
        let input = parse_quote! {
            enum X { A, B }
        };
        assert!(expand(input).is_err());
    }

    #[test]
    fn test_expand_rejects_a_tuple_struct() {
        let input = parse_quote! {
            struct X(bool);
        };
        assert!(expand(input).is_err());
    }

    #[test]
    fn test_expand_rejects_partial_container_attrs() {
        let input = parse_quote! {
            #[config(id = "logging")]
            struct X {}
        };
        assert!(expand(input).is_err());
    }

    #[test]
    fn test_expand_accepts_a_nested_type_with_no_container_attrs() {
        let input = parse_quote! {
            struct LoggingSettings {
                channel: Option<u64>,
            }
        };
        assert!(expand(input).is_ok());
    }

    #[test]
    fn test_expand_accepts_a_toggled_field() {
        let input = parse_quote! {
            struct X {
                logging: Toggled<LoggingSettings>,
            }
        };
        assert!(expand(input).is_ok());
    }

    #[test]
    fn test_expand_accepts_a_collection_field() {
        let input = parse_quote! {
            struct X {
                timers: BTreeMap<ChannelRef, ChannelAutoDelete>,
            }
        };
        assert!(expand(input).is_ok());
    }
}
