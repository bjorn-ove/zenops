use proc_macro::TokenStream;
use quote::quote;
use relative_path::RelativePath;
use syn::{Ident, LitStr, parse_macro_input};

/// Creates a `SafeRelativePath` without run-time cost by validating the string literal at compile time
#[proc_macro]
pub fn srpath(input: TokenStream) -> TokenStream {
    let arg = parse_macro_input!(input as LitStr).value();

    match RelativePath::from_path(&arg) {
        Ok(path) => {
            if safe_relative_path_validator::is_safe_relative_path(path) {
                quote! {
                    unsafe {
                        ::safe_relative_path::SafeRelativePath::new_unchecked_from_str(#arg)
                    }
                }
                .into()
            } else {
                syn::Error::new_spanned(&arg, "The relative path uses traversal")
                    .to_compile_error()
                    .into()
            }
        }
        Err(e) => syn::Error::new_spanned(&arg, e).to_compile_error().into(),
    }
}

#[proc_macro]
pub fn generate_is_valid_path_code(input: TokenStream) -> TokenStream {
    let ident = parse_macro_input!(input as Ident);

    quote! {
        ::std::convert::AsRef<::relative_path::RelativePath>
            .as_ref(#ident)
            .components()
            .scan(0, |level, c| {
                match c {
                    relative_path::Component::CurDir => (),
                    relative_path::Component::ParentDir => *level -= 1,
                    relative_path::Component::Normal(_) => *level += 1,
                }
                Some(*level)
            })
            .all(|level| level >= 0)
    }
    .into()
}
