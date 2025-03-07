//! Expansion of `#[zephyr::task(...)]`.

use std::fmt::Display;

use darling::FromMeta;
use darling::export::NestedMeta;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Expr, ExprLit, ItemFn, Lit, LitInt, ReturnType, Type,
    visit::{self, Visit},
};

#[derive(Debug, FromMeta, Default)]
struct Args {
    #[darling(default)]
    pool_size: Option<syn::Expr>,
    #[darling(default)]
    stack_size: Option<syn::Expr>,
}

pub fn run(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut errors = TokenStream::new();

    // If any of the steps for this macro fail, we still want to expand to an item that is as close
    // to the expected output as possible.  This helps out IDEs such that completions and other
    // related features keep working.
    let f: ItemFn = match syn::parse2(item.clone()) {
        Ok(x) => x,
        Err(e) => return token_stream_with_error(item, e),
    };

    let args = match NestedMeta::parse_meta_list(args) {
        Ok(x) => x,
        Err(e) => return token_stream_with_error(item, e),
    };

    let args = match Args::from_list(&args) {
        Ok(x) => x,
        Err(e) => {
            errors.extend(e.write_errors());
            Args::default()
        }
    };

    let pool_size = args.pool_size.unwrap_or(Expr::Lit(ExprLit {
        attrs: vec![],
        lit: Lit::Int(LitInt::new("1", Span::call_site())),
    }));

    let stack_size = args.stack_size.unwrap_or(Expr::Lit(ExprLit {
        attrs: vec![],
        // TODO: Instead of a default, require this.
        lit: Lit::Int(LitInt::new("2048", Span::call_site())),
    }));

    if !f.sig.asyncness.is_none() {
        error(&mut errors, &f.sig, "thread function must not be async");
    }

    if !f.sig.generics.params.is_empty() {
        error(&mut errors, &f.sig, "thread function must not be generic");
    }

    if !f.sig.generics.where_clause.is_none() {
        error(
            &mut errors,
            &f.sig,
            "thread function must not have `where` clauses",
        );
    }

    if !f.sig.abi.is_none() {
        error(
            &mut errors,
            &f.sig,
            "thread function must not have an ABI qualifier",
        );
    }

    if !f.sig.variadic.is_none() {
        error(&mut errors, &f.sig, "thread function must not be variadic");
    }

    match &f.sig.output {
        ReturnType::Default => {}
        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(tuple) if tuple.elems.is_empty() => {}
            Type::Never(_) => {}
            _ => error(
                &mut errors,
                &f.sig,
                "thread functions must either not return a value, return (), or return `!`",
            ),
        },
    }

    let mut args = Vec::new();
    let mut fargs = f.sig.inputs.clone();
    let mut inner_calling = Vec::new();
    let mut inner_args = Vec::new();

    for arg in fargs.iter_mut() {
        match arg {
            syn::FnArg::Receiver(_) => {
                error(
                    &mut errors,
                    arg,
                    "thread functions must not have `self` arguments",
                );
            }
            syn::FnArg::Typed(t) => {
                check_arg_ty(&mut errors, &t.ty);
                match t.pat.as_mut() {
                    syn::Pat::Ident(id) => {
                        id.mutability = None;
                        args.push((id.clone(), t.attrs.clone()));
                        inner_calling.push(quote! {
                            data.#id,
                        });
                        inner_args.push(quote! {#id,});
                    }
                    _ => {
                        error(
                            &mut errors,
                            arg,
                            "pattern matching in task arguments is not yet supported",
                        );
                    }
                }
            }
        }
    }

    let thread_ident = f.sig.ident.clone();
    let thread_inner_ident = format_ident!("__{}_thread", thread_ident);

    let mut thread_inner = f.clone();
    let visibility = thread_inner.vis.clone();
    thread_inner.vis = syn::Visibility::Inherited;
    thread_inner.sig.ident = thread_inner_ident.clone();

    // Assemble the original input arguments.
    let mut full_args = Vec::new();
    for (arg, cfgs) in &args {
        full_args.push(quote! {
            #(#cfgs)*
            #arg
        });
    }

    let mut thread_outer_body = quote! {
        const _ZEPHYR_INTERNAL_STACK_SIZE: usize = zephyr::thread::stack_len(#stack_size);
        const _ZEPHYR_INTERNAL_POOL_SIZE: usize = #pool_size;
        struct _ZephyrInternalArgs {
            // This depends on the argument syntax being valid as a struct definition, which should
            // be the case with the above constraints.
            #fargs
        }

        static THREAD: [zephyr::thread::ThreadData<_ZephyrInternalArgs>; _ZEPHYR_INTERNAL_POOL_SIZE]
            = [const { zephyr::thread::ThreadData::new() }; _ZEPHYR_INTERNAL_POOL_SIZE];
        #[unsafe(link_section = ".noinit.TODO_STACK")]
        static STACK: [zephyr::thread::ThreadStack<_ZEPHYR_INTERNAL_STACK_SIZE>; _ZEPHYR_INTERNAL_POOL_SIZE]
            = [const { zephyr::thread::ThreadStack::new() }; _ZEPHYR_INTERNAL_POOL_SIZE];

        extern "C" fn startup(
            arg0: *mut ::core::ffi::c_void,
            _: *mut ::core::ffi::c_void,
            _: *mut ::core::ffi::c_void,
        ) {
            let init = unsafe { &mut *(arg0 as *mut ::zephyr::thread::InitData<_ZephyrInternalArgs>) };
            let init = init.0.get();
            match unsafe { init.replace(None) } {
                None => {
                    ::core::panic!("Incorrect thread initialization");
                }
                Some(data) => {
                    #thread_inner_ident(#(#inner_calling)*);
                }
            }
        }

        zephyr::thread::ThreadData::acquire(
            &THREAD,
            &STACK,
            _ZephyrInternalArgs { #(#inner_args)* },
            Some(startup),
            0,
        )
    };

    let thread_outer_attrs = thread_inner.attrs.clone();

    if !errors.is_empty() {
        thread_outer_body = quote! {
            #[allow(unused_variables, unreachable_code)]
            let _x: ::zephyr::thread::ReadyThread = ::core::todo!();
            _x
        };
    }

    // Copy the generics + where clause to avoid more spurious errors.
    let generics = &f.sig.generics;
    let where_clause = &f.sig.generics.where_clause;

    quote! {
        // This is the user's thread function, renamed.
        #[doc(hidden)]
        #thread_inner

        #(#thread_outer_attrs)*
        #visibility fn #thread_ident #generics (#fargs) -> ::zephyr::thread::ReadyThread #where_clause {
            #thread_outer_body
        }

        #errors
    }
}

// Taken from embassy-executor-macros.
fn check_arg_ty(errors: &mut TokenStream, ty: &Type) {
    struct Visitor<'a> {
        errors: &'a mut TokenStream,
    }

    impl<'a, 'ast> Visit<'ast> for Visitor<'a> {
        fn visit_type_reference(&mut self, i: &'ast syn::TypeReference) {
            // only check for elided lifetime here.  If not elided, it is checked by
            // `visit_lifetime`.
            if i.lifetime.is_none() {
                error(
                    self.errors,
                    i.and_token,
                    "Arguments for threads must live forever. Try using the `'static` lifetime.",
                );
            }
            visit::visit_type_reference(self, i);
        }

        fn visit_lifetime(&mut self, i: &'ast syn::Lifetime) {
            if i.ident.to_string() != "static" {
                error(
                    self.errors,
                    i,
                    "Arguments for threads must live forever. Try using the `'static` lifetime.",
                );
            }
        }

        fn visit_type_impl_trait(&mut self, i: &'ast syn::TypeImplTrait) {
            error(
                self.errors,
                i,
                "`impl Trait` is not allowed in thread arguments. It is syntax sugar for generics, and threads cannot be generic.",
            );
        }
    }

    Visit::visit_type(&mut Visitor { errors }, ty);
}

// Utility borrowed from embassy-executor-macros.
pub fn token_stream_with_error(mut tokens: TokenStream, error: syn::Error) -> TokenStream {
    tokens.extend(error.into_compile_error());
    tokens
}

pub fn error<A: ToTokens, T: Display>(s: &mut TokenStream, obj: A, msg: T) {
    s.extend(syn::Error::new_spanned(obj.into_token_stream(), msg).into_compile_error())
}
