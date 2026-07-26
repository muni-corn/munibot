use syn::{GenericArgument, PathArguments, Type};

/// What a field's declared type means for schema generation.
///
/// A proc macro only ever sees syntax, never a fully resolved type, so this
/// is a name-based classification -- `Toggled`/`BTreeMap` are recognized by
/// their last path segment's identifier, not by actually resolving them to
/// `munibot_config::Toggled`/`std::collections::BTreeMap`. See
/// docs/notes/config-schema-implementation-notes.md for why there is
/// deliberately no third "plain nested section" shape in v1: without an
/// explicit opt-in it would be indistinguishable from `Leaf` (a
/// `ConfigChoice`-derived enum is also "some other type used directly").
// the Toggled/Collection payloads aren't read yet -- expand() (in
// configurable.rs) currently just rejects both shapes. real codegen using
// these fields lands in the next two commits.
#[allow(dead_code)]
pub enum FieldShape<'a> {
    /// `Toggled<T>` -> a toggled `Section`, with `T`'s own fields as its
    /// children.
    Toggled { settings_ty: &'a Type },
    /// `BTreeMap<K, V>` -> a `Collection` keyed by `K`, with `V`'s own
    /// fields as each item's shape.
    Collection {
        key_ty: &'a Type,
        value_ty: &'a Type,
    },
    /// Anything else -- assumed to implement `ConfigField` directly.
    Leaf,
}

pub fn classify_field_shape(ty: &Type) -> FieldShape<'_> {
    let Some((ident, arguments)) = last_segment(ty) else {
        return FieldShape::Leaf;
    };

    if ident == "Toggled"
        && let Some(settings_ty) = single_type_arg(arguments)
    {
        return FieldShape::Toggled { settings_ty };
    }

    if ident == "BTreeMap"
        && let Some((key_ty, value_ty)) = two_type_args(arguments)
    {
        return FieldShape::Collection { key_ty, value_ty };
    }

    FieldShape::Leaf
}

fn last_segment(ty: &Type) -> Option<(&syn::Ident, &PathArguments)> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    Some((&segment.ident, &segment.arguments))
}

fn type_args(arguments: &PathArguments) -> Option<impl Iterator<Item = &Type>> {
    let PathArguments::AngleBracketed(generic_args) = arguments else {
        return None;
    };
    Some(generic_args.args.iter().filter_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }))
}

fn single_type_arg(arguments: &PathArguments) -> Option<&Type> {
    let mut types = type_args(arguments)?;
    let only = types.next()?;
    types.next().is_none().then_some(only)
}

fn two_type_args(arguments: &PathArguments) -> Option<(&Type, &Type)> {
    let mut types = type_args(arguments)?;
    let first = types.next()?;
    let second = types.next()?;
    types.next().is_none().then_some((first, second))
}

#[cfg(test)]
mod tests {
    use syn::{Type, parse_quote};

    use super::FieldShape;
    use crate::shape::classify_field_shape;

    #[test]
    fn test_toggled_type_classifies_as_toggled() {
        let ty: Type = parse_quote!(Toggled<LoggingSettings>);
        let FieldShape::Toggled { settings_ty } = classify_field_shape(&ty) else {
            panic!("expected FieldShape::Toggled");
        };
        let expected: Type = parse_quote!(LoggingSettings);
        assert_eq!(*settings_ty, expected);
    }

    #[test]
    fn test_btreemap_type_classifies_as_collection() {
        let ty: Type = parse_quote!(BTreeMap<ChannelRef, ChannelAutoDelete>);
        let FieldShape::Collection { key_ty, value_ty } = classify_field_shape(&ty) else {
            panic!("expected FieldShape::Collection");
        };
        let expected_key: Type = parse_quote!(ChannelRef);
        let expected_value: Type = parse_quote!(ChannelAutoDelete);
        assert_eq!(*key_ty, expected_key);
        assert_eq!(*value_ty, expected_value);
    }

    #[test]
    fn test_plain_types_classify_as_leaf() {
        for ty in [
            parse_quote!(bool),
            parse_quote!(Option<ChannelRef>),
            parse_quote!(Vec<String>),
            parse_quote!(CleanupMode),
        ] as [Type; 4]
        {
            assert!(matches!(classify_field_shape(&ty), FieldShape::Leaf));
        }
    }

    #[test]
    fn test_toggled_with_wrong_generic_arg_count_is_leaf() {
        // `Toggled` with zero or two args isn't a real Toggled<T> -- treat
        // it as an (invalid, but not this function's job to reject) leaf
        // rather than panicking on the generic-arg assumption.
        let ty: Type = parse_quote!(Toggled<A, B>);
        assert!(matches!(classify_field_shape(&ty), FieldShape::Leaf));
    }
}
