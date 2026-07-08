use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Field, Fields, GenericArgument, Ident, PathArguments, Type,
    parse_macro_input,
};

#[proc_macro_derive(ResolveSecrets, attributes(secret))]
pub fn derive_resolve_secrets(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let body = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => {
                let futures = fields
                    .named
                    .iter()
                    .filter_map(struct_field_future)
                    .collect::<Vec<_>>();
                let declarations = futures.iter().map(|(_, declaration)| declaration);
                let future_names = futures.iter().map(|(name, _)| name);
                let join = join_futures(future_names);

                quote! {
                    #(#declarations)*
                    #join
                    Ok::<(), ::secret_resolve::Error>(())
                }
            }
            _ => quote! {
                compile_error!("ResolveSecrets only supports structs with named fields");
            },
        },
        Data::Enum(data) => {
            let arms = data.variants.iter().map(|variant| {
                let variant_name = &variant.ident;

                match &variant.fields {
                    Fields::Named(fields) => {
                        let field_patterns = fields
                            .named
                            .iter()
                            .map(|field| {
                                let field_name = field.ident.as_ref().expect("named field");
                                if is_secret(field) {
                                    quote! { #field_name }
                                } else {
                                    quote! { #field_name: _ }
                                }
                            })
                            .collect::<Vec<_>>();
                        let futures = fields
                            .named
                            .iter()
                            .filter_map(enum_named_field_future)
                            .collect::<Vec<_>>();
                        let declarations = futures.iter().map(|(_, declaration)| declaration);
                        let future_names = futures.iter().map(|(name, _)| name);
                        let join = join_futures(future_names);

                        quote! {
                            Self::#variant_name { #(#field_patterns),* } => {
                                #(#declarations)*
                                #join
                            }
                        }
                    }
                    Fields::Unit => quote! {
                        Self::#variant_name => {}
                    },
                    Fields::Unnamed(fields) => {
                        let field_names = fields
                            .unnamed
                            .iter()
                            .enumerate()
                            .map(|(index, _)| format_ident!("__field_{}", index))
                            .collect::<Vec<_>>();
                        let futures = fields
                            .unnamed
                            .iter()
                            .enumerate()
                            .map(enum_unnamed_field_future)
                            .collect::<Vec<_>>();
                        let declarations = futures.iter().map(|(_, declaration)| declaration);
                        let future_names = futures.iter().map(|(name, _)| name);
                        let join = join_futures(future_names);

                        quote! {
                            Self::#variant_name(#(#field_names),*) => {
                                #(#declarations)*
                                #join
                            }
                        }
                    }
                }
            });

            quote! {
                match self {
                    #(#arms),*
                }
                Ok::<(), ::secret_resolve::Error>(())
            }
        }
        Data::Union(_) => quote! {
            compile_error!("ResolveSecrets does not support unions");
        },
    };

    quote! {
        impl ::secret_resolve::ResolveSecrets for #name {
            async fn resolve_secrets(&mut self) -> Result<(), ::secret_resolve::Error> {
                #body
            }
        }
    }
    .into()
}

fn struct_field_future(field: &Field) -> Option<(Ident, proc_macro2::TokenStream)> {
    if !is_secret(field) {
        return None;
    }

    let field_name = field.ident.as_ref().expect("named field");
    let future_name = format_ident!("__resolve_{}", field_name);
    let resolver = resolver_for_type(&field.ty, quote! { &mut self.#field_name });

    Some((
        future_name.clone(),
        quote! {
            let #future_name = async {
                #resolver
            };
        },
    ))
}

fn enum_named_field_future(field: &Field) -> Option<(Ident, proc_macro2::TokenStream)> {
    if !is_secret(field) {
        return None;
    }

    let field_name = field.ident.as_ref().expect("named field");
    let future_name = format_ident!("__resolve_{}", field_name);
    let resolver = resolver_for_type(&field.ty, quote! { #field_name });

    Some((
        future_name.clone(),
        quote! {
            let #future_name = async {
                #resolver
            };
        },
    ))
}

fn enum_unnamed_field_future((index, field): (usize, &Field)) -> (Ident, proc_macro2::TokenStream) {
    let field_name = format_ident!("__field_{}", index);
    let future_name = format_ident!("__resolve_{}", index);
    let resolver = resolver_for_type(&field.ty, quote! { #field_name });

    (
        future_name.clone(),
        quote! {
            let #future_name = async {
                #resolver
            };
        },
    )
}

fn join_futures<'a>(names: impl Iterator<Item = &'a Ident>) -> proc_macro2::TokenStream {
    let names = names.collect::<Vec<_>>();

    if names.is_empty() {
        quote! {}
    } else {
        quote! {
            #(#names.await?;)*
        }
    }
}

fn resolver_for_type(ty: &Type, target: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    if is_string(ty) {
        quote! {
            let __value = #target;
            let __resolved = ::secret_resolve::resolve_secret(__value.as_str()).await?;
            *__value = __resolved;
            Ok::<(), ::secret_resolve::Error>(())
        }
    } else if let Some(inner) = option_inner(ty) {
        if is_string(inner) {
            quote! {
                let __option = #target;
                if let Some(__value) = __option.as_mut() {
                    let __resolved = ::secret_resolve::resolve_secret(__value.as_str()).await?;
                    *__value = __resolved;
                }
                Ok::<(), ::secret_resolve::Error>(())
            }
        } else {
            quote! {
                let __option = #target;
                if let Some(__value) = __option.as_mut() {
                    ::secret_resolve::ResolveSecrets::resolve_secrets(__value).await?;
                }
                Ok::<(), ::secret_resolve::Error>(())
            }
        }
    } else {
        quote! {
            ::secret_resolve::ResolveSecrets::resolve_secrets(#target).await?;
            Ok::<(), ::secret_resolve::Error>(())
        }
    }
}

fn is_secret(field: &Field) -> bool {
    field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("secret"))
}

fn is_string(ty: &Type) -> bool {
    path_ident(ty).is_some_and(|ident| ident == "String")
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let type_path = type_path(ty)?;
    let segment = type_path.path.segments.last()?;

    if segment.ident != "Option" {
        return None;
    }

    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };

    match args.args.first() {
        Some(GenericArgument::Type(inner)) => Some(inner),
        _ => None,
    }
}

fn path_ident(ty: &Type) -> Option<&Ident> {
    type_path(ty)?
        .path
        .segments
        .last()
        .map(|segment| &segment.ident)
}

fn type_path(ty: &Type) -> Option<&syn::TypePath> {
    match ty {
        Type::Path(type_path) if type_path.qself.is_none() => Some(type_path),
        _ => None,
    }
}
