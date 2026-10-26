// Type specifier parsing: handles all C type specifiers including struct/union/enum
// definitions, typedef names, and GNU extensions like typeof and _Complex.
//
// The main complexity here is that C allows type specifier tokens in any order
// (e.g., "long unsigned int" == "unsigned long int"), so we collect flags
// and resolve them at the end.

use super::ast::*;
use super::parse::{ModeKind, Parser};
use crate::common::source::Span;
use crate::common::types::AddressSpace;
use crate::frontend::lexer::token::TokenKind;

/// Collected type specifier flags during parsing.
///
/// C allows type specifier keywords in any order ("long unsigned int" ==
/// "unsigned long int"), so we accumulate boolean flags and a `long_count`
/// while scanning tokens, then resolve them into a concrete `TypeSpecifier`
/// via `Parser::resolve_type_flags`.
#[derive(Default)]
struct TypeSpecFlags {
    has_void: bool,
    has_bool: bool,
    has_float: bool,
    has_double: bool,
    has_complex: bool,
    has_char: bool,
    has_short: bool,
    has_int: bool,
    has_unsigned: bool,
    has_signed: bool,
    has_struct: bool,
    has_union: bool,
    has_enum: bool,
    has_typeof: bool,
    /// `typeof_unqual`: the expression type drops the named address space.
    typeof_unqual: bool,
    long_count: u32,
    typedef_name: Option<String>,
}

impl Parser {
    /// Parse a complete type specifier. Returns None if no type specifier found.
    ///
    /// Handles arbitrary ordering of type keywords (C allows "long unsigned int"
    /// or "unsigned long int"), struct/union/enum definitions, typedef names,
    /// typeof expressions, and _Complex types.
    pub(super) fn parse_type_specifier(&mut self) -> Option<TypeSpecifier> {
        self.skip_gcc_extensions();

        let mut flags = TypeSpecFlags::default();
        let mut mode_kind: Option<ModeKind> = None;
        let mut any_base_specifier = false;
        let mut any_storage_class = false;
        let mut saw_auto = false;

        // Collect qualifiers, storage classes, and type specifiers
        loop {
            match self.peek() {
                // Qualifiers
                TokenKind::Const => {
                    self.advance();
                    self.attrs.set_const(true);
                }
                // C23 `constexpr` (6.7.1): the object is const and its
                // initializer a constant expression.
                TokenKind::Constexpr => {
                    self.advance();
                    self.attrs.set_const(true);
                    self.attrs.set_constexpr(true);
                    if saw_auto && any_base_specifier {
                        self.attrs.mark_constexpr_last();
                    }
                    any_storage_class = true;
                }
                TokenKind::Volatile => {
                    self.advance();
                    self.attrs.set_volatile(true);
                }
                TokenKind::Restrict => {
                    self.advance();
                }
                TokenKind::Register => {
                    self.advance();
                    any_storage_class = true;
                }
                // `auto` shares `register`'s token position, and for a variable
                // both are type-irrelevant -- but at block scope on a *function*
                // `auto` is load-bearing: C11 6.7.1p5 makes every block-scope
                // function declaration *without* `auto` denote the
                // externally-visible entity, and GNU C uses `auto` to spell a
                // nested function, i.e. a distinct entity whose prototype must
                // never retype the outer one. Dropping the fact here is what
                // forced sema to guess with type inequality instead (F01).
                TokenKind::Auto => {
                    self.advance();
                    any_storage_class = true;
                    saw_auto = true;
                    self.attrs.set_nested_fn(true);
                }
                TokenKind::Noreturn => {
                    self.advance();
                    self.attrs.set_noreturn(true);
                }
                // GCC named address space qualifiers (__seg_gs / __seg_fs)
                TokenKind::SegGs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegGs;
                }
                TokenKind::SegFs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegFs;
                }
                // __auto_type - GCC extension: type inferred from initializer
                TokenKind::AutoType => {
                    self.advance();
                    return Some(TypeSpecifier::AutoType);
                }
                TokenKind::Inline => {
                    self.advance();
                    self.attrs.set_inline(true);
                }
                // Storage classes
                TokenKind::Static => {
                    self.advance();
                    self.attrs.set_static(true);
                    any_storage_class = true;
                }
                TokenKind::Extern => {
                    self.advance();
                    self.attrs.set_extern(true);
                    any_storage_class = true;
                }
                TokenKind::Typedef => {
                    self.advance();
                    self.attrs.set_typedef(true);
                    any_storage_class = true;
                }
                // Thread-local storage class (__thread / _Thread_local)
                TokenKind::ThreadLocal => {
                    self.advance();
                    self.attrs.set_thread_local(true);
                    any_storage_class = true;
                }
                // _Complex modifier
                TokenKind::Complex => {
                    self.advance();
                    flags.has_complex = true;
                    any_base_specifier = true;
                }
                // GNU extensions
                TokenKind::Attribute => {
                    let (_, aligned, mk, _, _) = self.parse_gcc_attributes();
                    mode_kind = mode_kind.or(mk);
                    if let Some(a) = aligned {
                        self.attrs.parsed_alignas =
                            Some(self.attrs.parsed_alignas.map_or(a, |prev| prev.max(a)));
                    }
                    // parse_gcc_attributes already sets self.attrs.parsing_constructor/destructor
                }
                TokenKind::Extension => {
                    self.advance();
                }
                // _Atomic as type specifier: _Atomic(type-name)
                // C11 §6.7.2.4: _Atomic(T) is a type specifier equivalent to T
                // with atomic qualification. Since we don't track atomic-ness,
                // we just parse and return the inner type.
                TokenKind::Atomic => {
                    self.advance();
                    // Atomic-ness is not tracked in types; the flag records the
                    // object qualifier (`DeclAttributes::object_atomic`).
                    self.attrs.set_atomic(true);
                    if matches!(self.peek(), TokenKind::LParen) {
                        let open = self.peek_span();
                        self.advance(); // consume '('
                        // Save and restore const qualifier across inner type parse
                        // to prevent leakage (e.g., _Atomic(const int) should not
                        // make the outer declaration const).
                        let saved_const = self.attrs.parsing_const();
                        self.attrs.set_const(false);
                        // Scope the pending address-space qualifier: the
                        // `_Atomic(type-name)` argument is a nested type-name,
                        // so a `*` in its abstract declarator must not steal
                        // an enclosing declaration's `__seg_gs`/`__seg_fs`,
                        // and the enclosing qualifier must be restored on
                        // every exit below.
                        let enclosing_address_space =
                            std::mem::take(&mut self.attrs.parsing_address_space);
                        let inner = self.parse_type_specifier();
                        self.attrs.set_const(saved_const);
                        if let Some(inner_type) = inner {
                            let result = self.parse_abstract_declarator_suffix(inner_type);
                            self.attrs.parsing_address_space = enclosing_address_space;
                            self.expect_closing(&TokenKind::RParen, open);
                            return Some(result);
                        }
                        // Fallback: if we can't parse a type, emit error and skip
                        let err_span = self.peek_span();
                        self.emit_error("expected type name in _Atomic(...)", err_span);
                        while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
                            self.advance();
                        }
                        self.consume_if(&TokenKind::RParen);
                        self.attrs.parsing_address_space = enclosing_address_space;
                        return Some(TypeSpecifier::Int);
                    }
                    // _Atomic without parens is a type qualifier; since we don't
                    // track atomic-ness, it falls through to continue collecting
                    // type specifiers.
                }
                // Alignas
                TokenKind::Alignas => {
                    self.advance();
                    if let Some(align) = self.parse_alignas_argument() {
                        self.attrs.parsed_alignas = Some(
                            self.attrs
                                .parsed_alignas
                                .map_or(align, |prev| prev.max(align)),
                        );
                    }
                }
                // Type specifier tokens
                TokenKind::Void => {
                    self.advance();
                    flags.has_void = true;
                    any_base_specifier = true;
                    break; // void can't combine with others
                }
                TokenKind::Char => {
                    self.advance();
                    flags.has_char = true;
                    any_base_specifier = true;
                    break; // char only combines with signed/unsigned
                }
                TokenKind::Short => {
                    self.advance();
                    flags.has_short = true;
                    any_base_specifier = true;
                }
                TokenKind::Int => {
                    self.advance();
                    flags.has_int = true;
                    any_base_specifier = true;
                }
                TokenKind::Long => {
                    self.advance();
                    flags.long_count += 1;
                    any_base_specifier = true;
                }
                TokenKind::Float => {
                    self.advance();
                    flags.has_float = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Double => {
                    self.advance();
                    flags.has_double = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Float128 => {
                    self.advance();
                    return Some(TypeSpecifier::Float128);
                }
                TokenKind::Decimal32 => {
                    self.advance();
                    return Some(TypeSpecifier::Decimal32);
                }
                TokenKind::Decimal64 => {
                    self.advance();
                    return Some(TypeSpecifier::Decimal64);
                }
                TokenKind::Decimal128 => {
                    self.advance();
                    return Some(TypeSpecifier::Decimal128);
                }
                TokenKind::Bool => {
                    self.advance();
                    flags.has_bool = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Signed => {
                    self.advance();
                    flags.has_signed = true;
                    any_base_specifier = true;
                }
                TokenKind::Unsigned => {
                    self.advance();
                    flags.has_unsigned = true;
                    any_base_specifier = true;
                }
                // __int128 can combine with signed/unsigned
                TokenKind::Int128 => {
                    let span = self.peek_span();
                    self.advance();
                    // GCC rejects __int128 on 32-bit targets
                    if crate::common::types::target_is_32bit() {
                        self.emit_error("__int128 is not supported on this target", span);
                        // Fall through with Int (to avoid cascading errors)
                        return Some(TypeSpecifier::Int);
                    }
                    // __int128 already implies signed unless unsigned is present
                    if flags.has_unsigned {
                        return Some(TypeSpecifier::UnsignedInt128);
                    } else {
                        return Some(TypeSpecifier::Int128);
                    }
                }
                // __uint128_t is always unsigned
                TokenKind::UInt128 => {
                    let span = self.peek_span();
                    self.advance();
                    // GCC rejects __uint128_t on 32-bit targets
                    if crate::common::types::target_is_32bit() {
                        self.emit_error("__uint128_t is not supported on this target", span);
                        return Some(TypeSpecifier::UnsignedInt);
                    }
                    return Some(TypeSpecifier::UnsignedInt128);
                }
                TokenKind::Struct => {
                    self.advance();
                    flags.has_struct = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Union => {
                    self.advance();
                    flags.has_union = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Enum => {
                    self.advance();
                    flags.has_enum = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Typeof | TokenKind::TypeofUnqual => {
                    flags.typeof_unqual = matches!(self.peek(), TokenKind::TypeofUnqual);
                    self.advance();
                    flags.has_typeof = true;
                    any_base_specifier = true;
                    break;
                }
                TokenKind::Builtin => {
                    if !any_base_specifier {
                        flags.typedef_name = Some("__builtin_va_list".to_string());
                        self.advance();
                        any_base_specifier = true;
                        break;
                    } else {
                        break;
                    }
                }
                // ISO C _FloatN/_FloatNx types (GCC 7+). On x86-64 these have
                // the same formats as existing types, so they map directly:
                // _Float32==float, _Float64==double, _Float32x==double,
                // _Float64x==long double (80-bit x87). (_Float128 has its own
                // TokenKind::Float128 and is a distinct binary128 type.)
                TokenKind::Identifier(name)
                    if matches!(
                        name.as_str(),
                        "_Float32" | "_Float64" | "_Float32x" | "_Float64x"
                    ) =>
                {
                    let nm = name.clone();
                    if !any_base_specifier {
                        self.advance();
                        any_base_specifier = true;
                        match nm.as_str() {
                            "_Float32" => {
                                flags.has_float = true;
                            }
                            "_Float64" | "_Float32x" => {
                                flags.has_double = true;
                            }
                            _ => {
                                flags.has_double = true;
                            }
                        }
                        break;
                    } else {
                        break;
                    }
                }
                TokenKind::Identifier(name)
                    if self.typedefs.contains(name) && !self.shadowed_typedefs.contains(name) =>
                {
                    if !any_base_specifier {
                        let name = name.clone();
                        // A typedef carrying a named space qualifies its uses
                        // as if `__seg_gs` were written there (C: the qualifier
                        // of `typedef int __seg_gs T` is part of T). An explicit
                        // keyword qualifier on the same declaration wins.
                        if self.attrs.parsing_address_space == AddressSpace::Default {
                            if let Some(&space) = self.typedef_object_spaces.get(&name) {
                                self.attrs.parsing_address_space = space;
                            }
                        }
                        // A typedef name is a synonym for its declarator text
                        // (C 6.7.8), so its qualifiers apply to the use. The base
                        // qualifier (`typedef volatile int *VPI`: the pointee) is
                        // the specifier's; the object qualifier (`typedef int
                        // *volatile VIP`) belongs to the declared object and is
                        // applied by the declarator.
                        self.attrs.set_typedef_use(true);
                        if self.typedef_base_volatile.contains(&name) {
                            self.attrs.set_volatile(true);
                        }
                        self.attrs.set_typedef_object_volatile(
                            self.typedef_object_volatile.contains(&name),
                        );
                        flags.typedef_name = Some(name);
                        self.advance();
                        any_base_specifier = true;
                        break;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        // After the main loop, collect trailing specifiers that can follow
        // certain base types (e.g., "short unsigned int", "double long", "float _Complex")
        self.collect_trailing_specifiers(&mut flags, &mut mode_kind);

        if !any_base_specifier {
            // C89 implicit int: if a storage class specifier was consumed but no
            // type specifier was found, the type defaults to int.
            // E.g., "static x = 5;" means "static int x = 5;",
            //       "register y;" means "register int y;".
            // C23 6.7.10: `auto` with no type specifier infers the type from
            // the initializer, exactly as GNU `__auto_type` (arrays and
            // functions decay). Before C23 it is implicit int.
            if saw_auto && self.c23 {
                return Some(TypeSpecifier::AutoType);
            }
            if any_storage_class {
                return Some(TypeSpecifier::Int);
            }
            return None;
        }

        // Resolve collected flags into a TypeSpecifier
        let base = self.resolve_type_flags(&flags);

        // Handle trailing _Complex, qualifiers, and storage classes after the base type
        let base = self.consume_trailing_qualifiers(base);

        // Apply __attribute__((mode(...))): transform type to the specified bit-width
        let base = if let Some(mk) = mode_kind {
            mk.apply(base)
        } else {
            base
        };

        Some(base)
    }

    /// Collect additional type specifier tokens that follow the initial base type.
    /// E.g., "short" can be followed by "unsigned int", "double" by "long",
    /// "float" by "_Complex".
    fn collect_trailing_specifiers(
        &mut self,
        flags: &mut TypeSpecFlags,
        mode_kind: &mut Option<ModeKind>,
    ) {
        if flags.has_char || flags.has_short || flags.has_int || flags.long_count > 0 {
            loop {
                match self.peek() {
                    TokenKind::Signed => {
                        self.advance();
                        flags.has_signed = true;
                    }
                    TokenKind::Unsigned => {
                        self.advance();
                        flags.has_unsigned = true;
                    }
                    TokenKind::Int => {
                        self.advance();
                        flags.has_int = true;
                    }
                    TokenKind::Long => {
                        self.advance();
                        flags.long_count += 1;
                    }
                    TokenKind::Short => {
                        self.advance();
                        flags.has_short = true;
                    }
                    TokenKind::Char => {
                        self.advance();
                        flags.has_char = true;
                    }
                    TokenKind::Complex => {
                        self.advance();
                        flags.has_complex = true;
                    }
                    // Qualifiers after the base type (`char const c`) are the
                    // declaration's, exactly as before it.
                    TokenKind::Const
                    | TokenKind::Constexpr
                    | TokenKind::Volatile
                    | TokenKind::Restrict => {
                        self.consume_qualifier_after_base();
                    }
                    TokenKind::SegGs => {
                        self.advance();
                        self.attrs.parsing_address_space = AddressSpace::SegGs;
                    }
                    TokenKind::SegFs => {
                        self.advance();
                        self.attrs.parsing_address_space = AddressSpace::SegFs;
                    }
                    TokenKind::Static => {
                        self.advance();
                        self.attrs.set_static(true);
                    }
                    TokenKind::Extern => {
                        self.advance();
                        self.attrs.set_extern(true);
                    }
                    TokenKind::Register => {
                        self.advance();
                    }
                    // `auto` shares `register`'s token position, and for a variable both are
                    // type-irrelevant -- but at block scope on a *function* `auto` is load-bearing:
                    // C11 6.7.1p5 makes every block-scope function declaration *without* `auto`
                    // denote the externally-visible entity, and GNU C uses `auto` to spell a
                    // nested function, i.e. a distinct entity whose prototype must never retype
                    // the outer one. Dropping the fact here is what forced sema to guess with type
                    // inequality instead (F01), and guessing is how the miscompile survived.
                    TokenKind::Auto => {
                        self.advance();
                        self.attrs.set_nested_fn(true);
                    }
                    TokenKind::ThreadLocal => {
                        self.advance();
                        self.attrs.set_thread_local(true);
                    }
                    TokenKind::Noreturn => {
                        self.advance();
                        self.attrs.set_noreturn(true);
                    }
                    TokenKind::Inline => {
                        self.advance();
                        self.attrs.set_inline(true);
                    }
                    TokenKind::Attribute => {
                        let (_, aligned, mk, _, _) = self.parse_gcc_attributes();
                        *mode_kind = mode_kind.or(mk);
                        if let Some(a) = aligned {
                            self.attrs.parsed_alignas =
                                Some(self.attrs.parsed_alignas.map_or(a, |prev| prev.max(a)));
                        }
                    }
                    TokenKind::Extension => {
                        self.advance();
                    }
                    _ => break,
                }
            }
        } else if flags.has_float {
            // "float" can be followed by "_Complex" and storage class / qualifiers
            loop {
                match self.peek() {
                    TokenKind::Complex => {
                        self.advance();
                        flags.has_complex = true;
                    }
                    // Qualifiers after the base type (`char const c`) are the
                    // declaration's, exactly as before it.
                    TokenKind::Const
                    | TokenKind::Constexpr
                    | TokenKind::Volatile
                    | TokenKind::Restrict => {
                        self.consume_qualifier_after_base();
                    }
                    TokenKind::SegGs => {
                        self.advance();
                        self.attrs.parsing_address_space = AddressSpace::SegGs;
                    }
                    TokenKind::SegFs => {
                        self.advance();
                        self.attrs.parsing_address_space = AddressSpace::SegFs;
                    }
                    TokenKind::Static => {
                        self.advance();
                        self.attrs.set_static(true);
                    }
                    TokenKind::Extern => {
                        self.advance();
                        self.attrs.set_extern(true);
                    }
                    TokenKind::Register => {
                        self.advance();
                    }
                    // `auto` shares `register`'s token position, and for a variable both are
                    // type-irrelevant -- but at block scope on a *function* `auto` is load-bearing:
                    // C11 6.7.1p5 makes every block-scope function declaration *without* `auto`
                    // denote the externally-visible entity, and GNU C uses `auto` to spell a
                    // nested function, i.e. a distinct entity whose prototype must never retype
                    // the outer one. Dropping the fact here is what forced sema to guess with type
                    // inequality instead (F01), and guessing is how the miscompile survived.
                    TokenKind::Auto => {
                        self.advance();
                        self.attrs.set_nested_fn(true);
                    }
                    TokenKind::ThreadLocal => {
                        self.advance();
                        self.attrs.set_thread_local(true);
                    }
                    TokenKind::Noreturn => {
                        self.advance();
                        self.attrs.set_noreturn(true);
                    }
                    TokenKind::Inline => {
                        self.advance();
                        self.attrs.set_inline(true);
                    }
                    TokenKind::Extension => {
                        self.advance();
                    }
                    _ => break,
                }
            }
        } else if flags.has_double {
            // "double" can be followed by "long", "_Complex", and storage class / qualifiers
            loop {
                match self.peek() {
                    TokenKind::Long => {
                        self.advance();
                        flags.long_count += 1;
                    }
                    TokenKind::Complex => {
                        self.advance();
                        flags.has_complex = true;
                    }
                    // Qualifiers after the base type (`char const c`) are the
                    // declaration's, exactly as before it.
                    TokenKind::Const
                    | TokenKind::Constexpr
                    | TokenKind::Volatile
                    | TokenKind::Restrict => {
                        self.consume_qualifier_after_base();
                    }
                    TokenKind::SegGs => {
                        self.advance();
                        self.attrs.parsing_address_space = AddressSpace::SegGs;
                    }
                    TokenKind::SegFs => {
                        self.advance();
                        self.attrs.parsing_address_space = AddressSpace::SegFs;
                    }
                    TokenKind::Static => {
                        self.advance();
                        self.attrs.set_static(true);
                    }
                    TokenKind::Extern => {
                        self.advance();
                        self.attrs.set_extern(true);
                    }
                    TokenKind::Register => {
                        self.advance();
                    }
                    // `auto` shares `register`'s token position, and for a variable both are
                    // type-irrelevant -- but at block scope on a *function* `auto` is load-bearing:
                    // C11 6.7.1p5 makes every block-scope function declaration *without* `auto`
                    // denote the externally-visible entity, and GNU C uses `auto` to spell a
                    // nested function, i.e. a distinct entity whose prototype must never retype
                    // the outer one. Dropping the fact here is what forced sema to guess with type
                    // inequality instead (F01), and guessing is how the miscompile survived.
                    TokenKind::Auto => {
                        self.advance();
                        self.attrs.set_nested_fn(true);
                    }
                    TokenKind::ThreadLocal => {
                        self.advance();
                        self.attrs.set_thread_local(true);
                    }
                    TokenKind::Noreturn => {
                        self.advance();
                        self.attrs.set_noreturn(true);
                    }
                    TokenKind::Inline => {
                        self.advance();
                        self.attrs.set_inline(true);
                    }
                    TokenKind::Extension => {
                        self.advance();
                    }
                    _ => break,
                }
            }
        }
    }

    /// Resolve the collected type specifier flags into a concrete TypeSpecifier.
    fn resolve_type_flags(&mut self, flags: &TypeSpecFlags) -> TypeSpecifier {
        let TypeSpecFlags {
            has_void,
            has_bool,
            has_float,
            has_double,
            has_complex,
            has_char,
            has_short,
            has_int: _,
            has_unsigned,
            has_signed,
            has_struct,
            has_union,
            has_enum,
            has_typeof,
            typeof_unqual,
            long_count,
            ref typedef_name,
        } = *flags;
        if has_void {
            TypeSpecifier::Void
        } else if has_bool {
            TypeSpecifier::Bool
        } else if has_float {
            if has_complex {
                TypeSpecifier::ComplexFloat
            } else {
                TypeSpecifier::Float
            }
        } else if has_double {
            if has_complex {
                if long_count > 0 {
                    TypeSpecifier::ComplexLongDouble
                } else {
                    TypeSpecifier::ComplexDouble
                }
            } else if long_count > 0 {
                TypeSpecifier::LongDouble
            } else {
                TypeSpecifier::Double
            }
        } else if has_complex && !has_struct && !has_union && !has_enum {
            // standalone _Complex defaults to _Complex double
            TypeSpecifier::ComplexDouble
        } else if has_struct {
            self.parse_struct_or_union(true)
        } else if has_union {
            self.parse_struct_or_union(false)
        } else if has_enum {
            self.parse_enum_specifier()
        } else if has_typeof {
            self.parse_typeof_specifier(typeof_unqual)
        } else if let Some(name) = typedef_name {
            TypeSpecifier::TypedefName(name.clone())
        } else if has_char {
            if has_unsigned {
                TypeSpecifier::UnsignedChar
            } else {
                // C11 6.7.2p5 + 6.2.5p15: plain char's signedness is
                // target-defined (unsigned on AArch64/RISC-V SysV, signed on
                // x86), yet char, signed char and unsigned char are three
                // distinct types. Resolve the representation here so no later
                // layer needs the flag; the alias variants carry identity.
                let unsigned_target = crate::common::types::char_is_unsigned();
                match (has_signed, unsigned_target) {
                    // plain `char`, unsigned target: unsigned layout, plain identity
                    (false, true) => TypeSpecifier::PlainUnsignedChar,
                    // plain `char`, signed target
                    (false, false) => TypeSpecifier::Char,
                    // explicit `signed char`, signed target: distinct from plain
                    (true, false) => TypeSpecifier::SignedChar,
                    // explicit `signed char`, unsigned target
                    (true, true) => TypeSpecifier::Char,
                }
            }
        } else if has_short {
            if has_unsigned {
                TypeSpecifier::UnsignedShort
            } else {
                TypeSpecifier::Short
            }
        } else if long_count >= 2 {
            if has_unsigned {
                TypeSpecifier::UnsignedLongLong
            } else {
                TypeSpecifier::LongLong
            }
        } else if long_count == 1 {
            if has_unsigned {
                TypeSpecifier::UnsignedLong
            } else {
                TypeSpecifier::Long
            }
        } else if has_unsigned {
            TypeSpecifier::UnsignedInt
        } else {
            // signed, int, or signed int
            TypeSpecifier::Int
        }
    }

    /// Parse a struct or union definition/reference.

    /// True when the type specifier names (or defines) an enum type.  The
    /// parser-level evaluator has no enum tag registry, so any enum-shaped
    /// alignof must be answered by sema (which resolves the compatible type
    /// from the completed definition).
    pub(super) fn type_spec_has_enum_tag(ts: &TypeSpecifier) -> bool {
        match ts {
            TypeSpecifier::Enum(..) => true,
            TypeSpecifier::Pointer(inner, _) => Self::type_spec_has_enum_tag(inner),
            TypeSpecifier::Array(inner, _) => Self::type_spec_has_enum_tag(inner),
            TypeSpecifier::TypeofType(inner) => Self::type_spec_has_enum_tag(inner),
            // A record with an enum field inherits the range-dependence: the
            // field's alignment is the completed enum's alignment.
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => fields
                .iter()
                .any(|f| Self::type_spec_has_enum_tag(&f.type_spec)),
            _ => false,
        }
    }

    fn parse_struct_or_union(&mut self, is_struct: bool) -> TypeSpecifier {
        // Type-budget frame: nested struct/union DEFINITIONS recurse one
        // frame per level (`struct { struct { ... int x; } s; }`); on
        // exhaustion return a placeholder WITHOUT consuming tokens -- the
        // budget diagnostic is already the TU verdict and the callers'
        // progress guarantees unwind the rest.
        if !self.enter_record_frame(self.peek_span()) {
            return TypeSpecifier::Int;
        }
        let (mut is_packed, mut struct_aligned, _, _, mut sso) = self.parse_gcc_attributes();
        let name = if let TokenKind::Identifier(n) = self.peek() {
            let n = n.clone();
            self.advance();
            Some(n)
        } else {
            None
        };
        let (packed2, aligned2, _, _, sso2) = self.parse_gcc_attributes();
        is_packed = is_packed || packed2;
        if aligned2.is_some() {
            struct_aligned = aligned2;
        }
        if sso2.is_some() {
            sso = sso2;
        }
        // An alignment attribute parsed BEFORE the `struct` keyword
        // (`__attribute__((aligned(N))) struct T { ... } member;`) belongs to
        // the DECLARATION that follows the struct body — GCC applies it to
        // the member declarator, never to the nested struct's own members.
        // Without this save/restore the pending value leaked into the body's
        // field parsing and landed on the nested struct's first field,
        // inflating the nested type's size and alignment
        // (gcc.c-torture/execute pr82210.c: `aligned(16) struct T { short
        // c; } a[size];` made sizeof(struct T) = 16 instead of 2, so the
        // VLA member spanned 8x the bytes and every later offset was wrong).
        let saved_pending_alignas = self.attrs.parsed_alignas.take();
        let saved_pending_alignas_type = self.attrs.parsed_alignas_type.take();
        let fields = if matches!(self.peek(), TokenKind::LBrace) {
            // Save and restore parsing_const across struct field parsing.
            // Field types may contain `const` (e.g., `const int *p`), and without
            // this save/restore the const from field types leaks into the outer
            // declaration, incorrectly marking non-const variables as const.
            let saved_const = self.attrs.parsing_const();
            let f = self.parse_struct_fields();
            self.attrs.set_const(saved_const);
            Some(f)
        } else {
            None
        };
        // Body parsed: hand the pending alignment back to the declarator
        // parsing that follows (post-body `__attribute__((aligned(N)))`
        // attributes read below apply to the struct type itself, matching
        // GCC's `struct T { ... } __attribute__((aligned(N)));` semantics).
        self.attrs.parsed_alignas = saved_pending_alignas;
        self.attrs.parsed_alignas_type = saved_pending_alignas_type;
        let (packed3, aligned3, _, _, sso3) = self.parse_gcc_attributes();
        is_packed = is_packed || packed3;
        if aligned3.is_some() {
            struct_aligned = aligned3;
        }
        if sso3.is_some() {
            sso = sso3;
        }
        // Apply current #pragma pack alignment to struct definition
        let max_field_align = self.pragma_pack_align;
        // Both supported targets (x86-64, i686) are little-endian, so the
        // storage order is reversed exactly when "big-endian" was requested.
        // When a big-endian target is ever added, this must become
        // `sso == Some(!target_is_big_endian)`.
        let reverse_sso = sso == Some(true);
        let ts = if is_struct {
            TypeSpecifier::Struct(
                name,
                fields,
                is_packed,
                max_field_align,
                struct_aligned,
                Some(reverse_sso),
            )
        } else {
            TypeSpecifier::Union(
                name,
                fields,
                is_packed,
                max_field_align,
                struct_aligned,
                Some(reverse_sso),
            )
        };
        // Record alignment for named struct/union definitions so that later
        // tag-only references (e.g., __alignof__(struct foo)) can look it up.
        // A definition containing an enum field is NOT cached: the field's
        // alignment follows the completed enum definition's value range
        // (sema's registry), which the parser cannot always see — caching a
        // 4-byte guess here made every later `_Alignof(struct WithEnum)` and
        // `_Static_assert(_Alignof(..) == 8)` answer 4 where GCC says 8.
        match &ts {
            TypeSpecifier::Struct(Some(tag), Some(_), ..)
            | TypeSpecifier::Union(Some(tag), Some(_), ..)
                if !Self::type_spec_has_enum_tag(&ts) =>
            {
                // Pass the tag-alignment map so nested NAMED records already
                // defined (parsed inside-out) resolve in O(1) instead of
                // re-walking the whole chain at every level. This is what
                // keeps deep struct nesting linear instead of quadratic.
                // Only exact alignments are cached; an unknown one must stay
                // unknown for later tag-only references too.
                match Self::alignof_type_spec(&ts, Some(&self.struct_tag_alignments)) {
                    Some(align) => {
                        self.struct_tag_alignments.insert(tag.clone(), align);
                    }
                    None => {
                        self.struct_tag_alignments.remove(tag);
                    }
                }
            }
            _ => {}
        }
        self.exit_parser_frame();
        ts
    }

    /// Parse an enum definition/reference.
    fn parse_enum_specifier(&mut self) -> TypeSpecifier {
        let (mut is_packed, _, _, _, _) = self.parse_gcc_attributes();
        let name = if let TokenKind::Identifier(n) = self.peek() {
            let n = n.clone();
            self.advance();
            Some(n)
        } else {
            None
        };
        let (packed2, _, _, _, _) = self.parse_gcc_attributes();
        is_packed = is_packed || packed2;
        let underlying = self.parse_enum_underlying_type();
        let variants = if matches!(self.peek(), TokenKind::LBrace) {
            let v = self.parse_enum_variants();
            // Register enum constant values so that later constant expressions
            // (e.g., in __attribute__((aligned(1 << ENUM_CONST)))) can resolve them.
            self.register_enum_constants(&v);
            Some(v)
        } else {
            None
        };
        let (packed3, _, _, _, _) = self.parse_gcc_attributes();
        is_packed = is_packed || packed3;
        TypeSpecifier::Enum(name, variants, is_packed, underlying)
    }

    /// C23 6.7.2.2: the optional `: specifier-qualifier-list` naming an
    /// enum's fixed underlying type. A `:` not followed by a type specifier
    /// is a bit-field width (`struct S { enum E : 3; };`) and is left alone.
    /// The type may only accompany a definition or stand alone (`enum E :
    /// int;`); validity of the type itself is sema's (GCC 16.2 wording).
    fn parse_enum_underlying_type(&mut self) -> Option<Box<TypeSpecifier>> {
        if !matches!(self.peek(), TokenKind::Colon) {
            return None;
        }
        let colon = self.pos;
        let span = self.peek_span();
        self.advance();
        if !self.is_type_specifier() {
            self.pos = colon;
            return None;
        }
        // The list's qualifiers and specifiers are its own, not those of the
        // declaration the enum specifier sits in.
        let enclosing = self.attrs.flags;
        let underlying = self.parse_type_specifier();
        self.attrs.flags = enclosing;
        if !matches!(self.peek(), TokenKind::LBrace | TokenKind::Semicolon) {
            self.emit_error("'enum' underlying type may not be specified here", span);
        }
        underlying.map(Box::new)
    }

    /// Parse typeof(expr) or typeof(type-name).
    fn parse_typeof_specifier(&mut self, unqual: bool) -> TypeSpecifier {
        let open = self.peek_span();
        self.expect_context(&TokenKind::LParen, "after 'typeof'");
        // Save attrs.flags so that storage-class specifiers from declarations
        // inside a statement expression (e.g., `typeof(({ extern void f(); x; }))`)
        // don't leak into the outer declaration.
        let saved_flags = self.attrs.flags;
        let save = self.pos;
        // Try parsing as a type first. parse_nested_type_name scopes the
        // pending address-space qualifier: an enclosing declaration's
        // `__seg_gs`/`__seg_fs` must survive a `*` inside this argument
        // (the kernel's `extern __seg_gs __typeof__(struct irq_stack *) x;`
        // shape), while a qualifier the argument sets itself still reaches
        // its own declarator. The restore happens inside the helper before
        // it returns, so the backtrack path below also sees the enclosing
        // state again.
        if self.is_type_specifier() {
            if let Some(result_type) = self.parse_nested_type_name() {
                if matches!(self.peek(), TokenKind::RParen) {
                    self.advance();
                    self.attrs.flags = saved_flags;
                    return TypeSpecifier::TypeofType(Box::new(result_type));
                }
            }
            // Didn't work as type, backtrack
            self.pos = save;
            self.attrs.flags = saved_flags;
            self.expect_context(&TokenKind::LParen, "after 'typeof'");
        }
        // Parse as expression
        let expr = self.parse_expr();
        self.expect_closing(&TokenKind::RParen, open);
        self.attrs.flags = saved_flags;
        TypeSpecifier::Typeof(Box::new(expr), unqual)
    }

    /// Consume trailing qualifiers and _Complex that may follow a resolved base type.
    /// C allows "int static x;" and "double _Complex".
    /// A `const`, `constexpr`, `volatile` or `restrict` after a base type
    /// specifier. An `auto` before it is a storage class by now, so a
    /// `constexpr` here is the later of the two (GCC 16.2 diagnostic order).
    fn consume_qualifier_after_base(&mut self) {
        match self.peek() {
            TokenKind::Const => self.attrs.set_const(true),
            TokenKind::Volatile => self.attrs.set_volatile(true),
            TokenKind::Constexpr => {
                self.attrs.set_const(true);
                self.attrs.set_constexpr(true);
                if self.attrs.parsing_nested_fn() {
                    self.attrs.mark_constexpr_last();
                }
            }
            _ => {}
        }
        self.advance();
    }

    fn consume_trailing_qualifiers(&mut self, mut base: TypeSpecifier) -> TypeSpecifier {
        loop {
            match self.peek() {
                TokenKind::Complex => {
                    self.advance();
                    base = match base {
                        TypeSpecifier::Float => TypeSpecifier::ComplexFloat,
                        TypeSpecifier::Double => TypeSpecifier::ComplexDouble,
                        TypeSpecifier::LongDouble => TypeSpecifier::ComplexLongDouble,
                        _ => TypeSpecifier::ComplexDouble,
                    };
                }
                // Qualifiers after the base type (`char const c`) are the
                // declaration's, exactly as before it.
                TokenKind::Const
                | TokenKind::Constexpr
                | TokenKind::Volatile
                | TokenKind::Restrict => {
                    self.consume_qualifier_after_base();
                }
                TokenKind::Static => {
                    self.advance();
                    self.attrs.set_static(true);
                }
                TokenKind::Extern => {
                    self.advance();
                    self.attrs.set_extern(true);
                }
                TokenKind::Register => {
                    self.advance();
                }
                // `auto` shares `register`'s token position, and for a variable both are
                // type-irrelevant -- but at block scope on a *function* `auto` is load-bearing:
                // C11 6.7.1p5 makes every block-scope function declaration *without* `auto`
                // denote the externally-visible entity, and GNU C uses `auto` to spell a
                // nested function, i.e. a distinct entity whose prototype must never retype
                // the outer one. Dropping the fact here is what forced sema to guess with type
                // inequality instead (F01), and guessing is how the miscompile survived.
                TokenKind::Auto => {
                    self.advance();
                    self.attrs.set_nested_fn(true);
                }
                TokenKind::ThreadLocal => {
                    self.advance();
                    self.attrs.set_thread_local(true);
                }
                TokenKind::Noreturn => {
                    self.advance();
                    self.attrs.set_noreturn(true);
                }
                TokenKind::Inline => {
                    self.advance();
                    self.attrs.set_inline(true);
                }
                TokenKind::Attribute => {
                    let (_, aligned, _, _, _) = self.parse_gcc_attributes();
                    if let Some(a) = aligned {
                        self.attrs.parsed_alignas =
                            Some(self.attrs.parsed_alignas.map_or(a, |prev| prev.max(a)));
                    }
                }
                TokenKind::Extension => {
                    self.advance();
                }
                TokenKind::SegGs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegGs;
                }
                TokenKind::SegFs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegFs;
                }
                _ => break,
            }
        }
        base
    }

    /// Parse struct or union field declarations inside braces.
    pub(super) fn parse_struct_fields(&mut self) -> Vec<StructFieldDecl> {
        let mut fields = Vec::with_capacity(8);
        let open = self.peek_span();
        // The members' `volatile` qualifiers must not outlive the body:
        // `struct S { volatile int a; } s;` leaves `s` non-volatile.
        let outer_volatile = self.attrs.parsing_volatile();
        let outer_counted_by = self.attrs.parsing_counted_by.take();
        self.expect_context(&TokenKind::LBrace, "for struct/union body");
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            // Budget termination guarantee: once the nesting budget is
            // exhausted, every level unwinds immediately instead of
            // grinding through the remaining tokens (the budget
            // diagnostic is the TU verdict).
            if self.nesting_budget_diagnosed {
                break;
            }
            let pos_before = self.pos;
            self.attrs.parsing_counted_by = None;
            let (pre_spec_packed, pre_spec_align, _, _, _) = self.parse_gcc_attributes();
            if let Some(a) = pre_spec_align {
                self.attrs.parsed_alignas =
                    Some(self.attrs.parsed_alignas.map_or(a, |prev| prev.max(a)));
            }
            self.skip_gcc_extensions();
            // Per-field reset: an `address_space(__seg_gs)` attribute parsed
            // for one field must not leak into the next (fields without a
            // declarator — anonymous/bitfield arms — never snapshot it).
            self.attrs.parsing_address_space = AddressSpace::Default;
            // Likewise `volatile`: a previous member's qualifier must not
            // make this member volatile (`volatile int a; int b;`).
            self.attrs.set_volatile(false);
            if matches!(self.peek(), TokenKind::Semicolon) {
                self.advance();
                continue;
            }
            // C11 6.7.2.1: _Static_assert is allowed as a struct-declaration.
            // Struct members carry no AST slot for assertions, so the
            // assertion is queued and emitted after the enclosing item, where
            // sema decides it like any other.
            if matches!(self.peek(), TokenKind::StaticAssert) {
                let deferred = self.parse_static_assert();
                // Speculative parses (nested-function and cast lookahead)
                // can parse one member list twice inside an item; the
                // queue drains only between items, so the span identifies
                // a re-parse exactly.
                if !self
                    .deferred_static_asserts
                    .iter()
                    .any(|queued| queued.span == deferred.span)
                {
                    self.deferred_static_asserts.push(deferred);
                }
                continue;
            }
            if let Some(type_spec) = self.parse_type_specifier() {
                if matches!(self.peek(), TokenKind::Semicolon) {
                    // Anonymous field (e.g., anonymous struct/union)
                    let alignment = self.attrs.parsed_alignas.take();
                    let alignas_type = self.attrs.parsed_alignas_type.take();
                    fields.push(StructFieldDecl {
                        type_spec,
                        name: None,
                        bit_width: None,
                        derived: Vec::new(),
                        alignment,
                        alignas_type,
                        is_packed: pre_spec_packed,
                        base_volatile: false,
                        object_volatile: false,
                        counted_by: None,
                    });
                } else {
                    self.parse_struct_field_declarators(&type_spec, &mut fields, pre_spec_packed);
                }
                self.skip_gcc_extensions();
                self.expect_after(&TokenKind::Semicolon, "after struct field declaration");
            } else {
                self.advance(); // skip unknown
            }
            // Universal progress guarantee: a field parse that consumed
            // nothing (e.g. a budget-exhausted placeholder type) must not
            // be retried on the same token.
            if self.pos == pos_before {
                self.advance();
            }
        }
        self.expect_closing(&TokenKind::RBrace, open);
        self.attrs.set_volatile(outer_volatile);
        self.attrs.parsing_counted_by = outer_counted_by;
        fields
    }

    /// Reject a named address space on a struct or union field object, as GCC
    /// 14 does (`'__seg_gs' specified for structure field 'f'`). Such a field
    /// would need per-object storage that a record cannot give it, so lccc
    /// used to drop the qualifier silently and the field read generic memory.
    ///
    /// A qualifier is legal only where it reaches the field through an
    /// indirection: `int __seg_gs *p`, `int __seg_gs *arr[4]`,
    /// `int __seg_gs (*fp)(void)` qualify the pointee or the return type, not
    /// the field object. `int __seg_gs f`, `int __seg_gs a[4]`, and a bit-field
    /// all qualify the field object itself. A qualifier written after the
    /// declarator's last `*` (`int *__seg_gs p`) names the pointer object and
    /// is always rejected.
    fn check_struct_field_space(
        &mut self,
        base_space: AddressSpace,
        object_space: AddressSpace,
        name: Option<&str>,
        derived: &[DerivedDeclarator],
        field_type: &TypeSpecifier,
        span: Span,
    ) {
        let has_indirection = matches!(
            field_type,
            TypeSpecifier::Pointer(..) | TypeSpecifier::FunctionPointer(..)
        ) || derived.iter().any(|d| {
            matches!(
                d,
                DerivedDeclarator::Pointer(_)
                    | DerivedDeclarator::Function(..)
                    | DerivedDeclarator::FunctionPointer(..)
            )
        });
        let offending = if object_space != AddressSpace::Default {
            object_space
        } else if base_space != AddressSpace::Default && !has_indirection {
            base_space
        } else {
            return;
        };
        let message = match name {
            Some(n) => format!("'{}' specified for structure field '{}'", offending, n),
            None => format!("'{}' specified for structure field", offending),
        };
        self.emit_error(message, span);
    }

    /// Parse one or more declarators for a struct field using the general
    /// declarator parser. This correctly handles complex declarators like
    /// function pointers returning function pointers (e.g.,
    /// `void (*(*xDlSym)(sqlite3_vfs*, void*, const char *))(void)`).
    fn parse_struct_field_declarators(
        &mut self,
        type_spec: &TypeSpecifier,
        fields: &mut Vec<StructFieldDecl>,
        pre_spec_packed: bool,
    ) {
        // Capture _Alignas value that was parsed during type specifier parsing
        let mut alignas_from_type = self.attrs.parsed_alignas.take();
        let mut packed_from_type = pre_spec_packed;

        // Consume post-type qualifiers that may appear between type and declarator
        // in struct field declarations: e.g., "char _Alignas(32) c;"
        self.consume_struct_field_qualifiers(&mut alignas_from_type, &mut packed_from_type);
        // A type-form `_Alignas` the parser could not size (in either
        // position) is resolved by the layout builders.
        let alignas_type = self.attrs.parsed_alignas_type.take();

        // Snapshot the field's address-space qualifier NOW (before the
        // declarator parser runs — the abstract-declarator path consumes and
        // clears `parsing_address_space`, so reading it afterwards loses the
        // qualifier). The kernel's percpu fields depend on this:
        // `unsigned int __percpu *read_count;` must type the field as a
        // %gs-qualified pointer, otherwise assigning `alloc_percpu(int)`
        // (a `__seg_gs int *`) trips "incompatible pointer types".
        let field_addr_space = self.attrs.parsing_address_space;
        self.attrs.parsing_address_space = AddressSpace::Default;
        // `volatile` written in the specifier applies to every declarator. A
        // star's `volatile` in an earlier declarator (`volatile int *a, b;`
        // style) must not leak into the next one, so each declarator restarts
        // from the specifier's flag.
        let spec_volatile = self.attrs.parsing_volatile();

        loop {
            self.attrs.set_volatile(spec_volatile);
            // Handle unnamed bitfield: `: constant-expr`
            if matches!(self.peek(), TokenKind::Colon) {
                self.advance();
                // Use parse_assignment_expr to avoid consuming comma as comma-operator.
                // Bitfield width is a constant expression (no comma operator at top level).
                let bit_width = Some(Box::new(self.parse_assignment_expr()));
                let (extra_packed, extra_aligned, _, _, _) = self.parse_gcc_attributes();
                let alignment = [extra_aligned, alignas_from_type]
                    .into_iter()
                    .flatten()
                    .max();
                let colon_span = self.peek_span();
                self.check_struct_field_space(
                    field_addr_space,
                    AddressSpace::Default,
                    None,
                    &[],
                    &TypeSpecifier::Void,
                    colon_span,
                );
                fields.push(StructFieldDecl {
                    type_spec: type_spec.clone(),
                    name: None,
                    bit_width,
                    derived: Vec::new(),
                    alignment,
                    alignas_type: alignas_type.clone(),
                    is_packed: packed_from_type || extra_packed,
                    base_volatile: false,
                    object_volatile: false,
                    counted_by: None,
                });
                if !self.consume_if(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            // Use the general-purpose declarator parser. This handles all cases:
            // simple pointers, arrays, function pointers, and nested function
            // pointer declarators of arbitrary depth.
            // parse_declarator_with_attrs also consumes trailing __attribute__ and
            // returns the aligned value from __attribute__((aligned(N))).
            let declarator_span = self.peek_span();
            let pre_decl_counted_by = self.attrs.parsing_counted_by.take();
            self.attrs.declarator_object_space = AddressSpace::Default;
            let (name, derived, _, _, decl_aligned, decl_packed) =
                self.parse_declarator_with_attrs();
            // The declarator's volatile qualifiers (see `declarator_base_volatile`).
            let decl_base_volatile = self.attrs.declarator_base_volatile;
            let decl_object_volatile = self.attrs.declarator_object_volatile;
            let declarator_object_space = std::mem::replace(
                &mut self.attrs.declarator_object_space,
                AddressSpace::Default,
            );

            // Parse optional bitfield width (constant-expression, not full expr with comma)
            let bit_width = if self.consume_if(&TokenKind::Colon) {
                Some(Box::new(self.parse_assignment_expr()))
            } else {
                None
            };

            // Parse any additional trailing GCC __attribute__ (e.g., after bitfield width)
            let (extra_packed, extra_aligned, _, _, _) = self.parse_gcc_attributes();
            let counted_by = self.attrs.parsing_counted_by.take().or(pre_decl_counted_by);

            // The strictest of every alignment source applies (C11 6.7.5p6;
            // GCC's `aligned` attribute on a member only ever raises it).
            let alignment = [decl_aligned, extra_aligned, alignas_from_type]
                .into_iter()
                .flatten()
                .max();
            let is_packed = packed_from_type || decl_packed || extra_packed;

            // For backward compatibility with downstream code that reads type_spec
            // directly, fold simple derived declarators (pointers, arrays) into
            // type_spec. Only use the derived field for complex cases with function
            // pointers that require build_full_ctype().
            let (mut field_type, field_derived) = Self::fold_simple_derived(type_spec, &derived);
            self.check_struct_field_space(
                field_addr_space,
                declarator_object_space,
                name.as_deref(),
                &derived,
                &field_type,
                declarator_span,
            );
            // Attach the address-space qualifier (e.g. `__percpu`) to the
            // pointer the qualifier was written BEFORE. GCC binds such a
            // qualifier to the first (innermost) star: `unsigned int __percpu
            // *read_count` dereferences with %gs, `int __seg_gs **f` keeps
            // the outer dereference generic and puts %gs on the second
            // (`**f`), and `void __percpu *pptrs[]` (DECLARE_FLEX_ARRAY)
            // makes every ELEMENT a %gs-qualified pointer, so the slot sits
            // under the array dimensions.
            if field_addr_space != AddressSpace::Default {
                field_type = Self::attach_before_star_space(field_type, field_addr_space);
            }

            fields.push(StructFieldDecl {
                type_spec: field_type,
                name,
                bit_width,
                derived: field_derived,
                alignment,
                alignas_type: alignas_type.clone(),
                is_packed,
                base_volatile: decl_base_volatile,
                object_volatile: decl_object_volatile,
                counted_by,
            });

            if !self.consume_if(&TokenKind::Comma) {
                break;
            }
        }
    }

    /// Consume qualifiers that may appear between type specifier and declarator
    /// in struct field declarations. Handles _Alignas, const, volatile, etc.
    fn consume_struct_field_qualifiers(
        &mut self,
        alignas: &mut Option<usize>,
        is_packed: &mut bool,
    ) {
        loop {
            match self.peek() {
                TokenKind::Const
                | TokenKind::Constexpr
                | TokenKind::Volatile
                | TokenKind::Restrict => {
                    self.advance();
                }
                TokenKind::SegGs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegGs;
                }
                TokenKind::SegFs => {
                    self.advance();
                    self.attrs.parsing_address_space = AddressSpace::SegFs;
                }
                TokenKind::Alignas => {
                    self.advance();
                    if let Some(align) = self.parse_alignas_argument() {
                        *alignas = Some(alignas.map_or(align, |prev| prev.max(align)));
                    }
                }
                TokenKind::Attribute => {
                    let (attr_packed, attr_aligned, _, _, _) = self.parse_gcc_attributes();
                    *is_packed = *is_packed || attr_packed;
                    if let Some(a) = attr_aligned {
                        *alignas = Some(alignas.map_or(a, |prev| prev.max(a)));
                    }
                }
                TokenKind::Extension => {
                    self.advance();
                }
                _ => break,
            }
        }
    }

    /// For simple derived declarators (just pointers and/or arrays), fold them
    /// into the TypeSpecifier directly to maintain backward compatibility with
    /// downstream code. For complex cases (function pointers), return the
    /// derived list for downstream to process with build_full_ctype().
    /// Bind a BEFORE-the-star address-space qualifier to the pointer it
    /// applies to. GCC attaches such a qualifier to the first (innermost)
    /// star, reached through any array dimensions and outer pointer levels:
    /// `unsigned int __percpu *read_count` dereferences with %gs, `int
    /// __seg_gs **f` keeps the outer dereference generic and puts %gs on the
    /// second (`**f`), and `void __percpu *pptrs[]` (DECLARE_FLEX_ARRAY)
    /// makes every ELEMENT a %gs-qualified pointer, so the slot sits under
    /// the array dimensions. A type without a bindable pointer is returned
    /// unchanged (the qualifier was rejected by check_struct_field_space).
    fn attach_before_star_space(t: TypeSpecifier, space: AddressSpace) -> TypeSpecifier {
        match t {
            TypeSpecifier::Array(elem, n) => {
                TypeSpecifier::Array(Box::new(Self::attach_before_star_space(*elem, space)), n)
            }
            TypeSpecifier::Pointer(pointee, s)
                if matches!(&*pointee, TypeSpecifier::Pointer(..)) =>
            {
                TypeSpecifier::Pointer(Box::new(Self::attach_before_star_space(*pointee, space)), s)
            }
            TypeSpecifier::Pointer(pointee, _) => TypeSpecifier::Pointer(pointee, space),
            other => other,
        }
    }

    fn fold_simple_derived(
        base: &TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> (TypeSpecifier, Vec<DerivedDeclarator>) {
        // If derived contains any function-related declarators, pass it through
        let has_function = derived.iter().any(|d| {
            matches!(
                d,
                DerivedDeclarator::Function(_, _) | DerivedDeclarator::FunctionPointer(_, _)
            )
        });

        if has_function {
            return (base.clone(), derived.to_vec());
        }

        if derived.is_empty() {
            return (base.clone(), Vec::new());
        }

        // Simple case: only Pointer and Array declarators. Fold into type_spec.
        // The space mapping replicates `build_full_ctype_with_base` exactly
        // (its `pointee_space_before`): the wrapper built at step i takes the
        // space stored at derived[i-1] — the qualifier written after star j
        // says the pointer values that the next-outer star points at live in
        // that space (`struct sd * __seg_gs *sd` = the kernel's
        // `struct sched_domain * __percpu *`: the first dereference reads %gs).
        // The old code dropped every space, so field types lost %gs and
        // `free_percpu(sdd->sd)` failed that GCC accepts.
        let mut result = base.clone();
        let mut pending_space = AddressSpace::Default;
        let mut i = 0;
        while i < derived.len() {
            match &derived[i] {
                DerivedDeclarator::Pointer(space) => {
                    result = TypeSpecifier::Pointer(Box::new(result), pending_space);
                    pending_space = *space;
                    i += 1;
                }
                DerivedDeclarator::Array(_) => {
                    // Collect consecutive array dims, apply in reverse (innermost first)
                    let start = i;
                    while i < derived.len() && matches!(&derived[i], DerivedDeclarator::Array(_)) {
                        i += 1;
                    }
                    for j in (start..i).rev() {
                        if let DerivedDeclarator::Array(size_expr) = &derived[j] {
                            result = TypeSpecifier::Array(Box::new(result), size_expr.clone());
                        }
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }
        (result, Vec::new())
    }

    /// Parse enum variant declarations inside braces.
    pub(super) fn parse_enum_variants(&mut self) -> Vec<EnumVariant> {
        let mut variants = Vec::with_capacity(8);
        let open = self.peek_span();
        self.expect_context(&TokenKind::LBrace, "for enum body");
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            if let TokenKind::Identifier(name) = self.peek() {
                let name = name.clone();
                let span = self.peek_span();
                self.advance();
                // Enumerator attributes belong to this enumerator, not the
                // enclosing declaration. Consume them before looking for `=`;
                // otherwise their identifiers (and the initializer's name)
                // become bogus enumerators and overwrite earlier constants.
                let enclosing_attrs = std::mem::take(&mut self.attrs);
                loop {
                    let before = self.pos;
                    self.parse_gcc_attributes();
                    self.skip_c23_attribute_lists();
                    if self.pos == before {
                        break;
                    }
                }
                self.attrs = enclosing_attrs;
                let value = if self.consume_if(&TokenKind::Assign) {
                    Some(Box::new(self.parse_assignment_expr()))
                } else {
                    None
                };
                variants.push(EnumVariant { name, value, span });
                self.consume_if(&TokenKind::Comma);
            } else {
                self.advance();
            }
        }
        self.expect_closing(&TokenKind::RBrace, open);
        variants
    }

    /// Register enum constant values from parsed variants into the parser's
    /// enum_constants map. This allows later constant expressions (e.g., in
    /// __attribute__((aligned(1 << ENUM_CONST)))) to resolve these identifiers.
    pub(super) fn register_enum_constants(&mut self, variants: &[super::ast::EnumVariant]) {
        let mut next_value: Option<i64> = Some(0);
        for variant in variants {
            let evaluated = if let Some(ref expr) = variant.value {
                // Evaluate the explicit value expression.
                // Use eval_const_int_expr_with_enums so references to previously
                // defined enum constants are resolved.
                let tag_aligns = if self.struct_tag_alignments.is_empty() {
                    None
                } else {
                    Some(&self.struct_tag_alignments)
                };
                Self::eval_const_int_expr_with_enums(expr, Some(&self.enum_constants), tag_aligns)
            } else {
                next_value
            };
            if self.block_depth > 0 {
                self.hide_enum_constant(&variant.name);
            }
            if let Some(val) = evaluated {
                self.enum_constants.insert(variant.name.clone(), val);
                next_value = Some(val + 1);
            } else {
                // Value not evaluable at parse time (e.g., sizeof(struct)).
                // Don't store in the value map, but record the name so that
                // expr_has_non_const_identifier knows this is a constant (not a variable).
                self.unevaluable_enum_constants.insert(variant.name.clone());
                next_value = None;
            }
        }
    }

    /// Parse a type-name for __builtin_va_arg: type-specifier + abstract declarator.
    pub(super) fn parse_va_arg_type(&mut self) -> TypeSpecifier {
        if let Some(type_spec) = self.parse_type_specifier() {
            let mut result_type = type_spec;
            // Parse pointer declarators. Bounded: see POINTER_DEPTH_BUDGET.
            // These loops build the nested TypeSpecifier directly, so unlike
            // the declarator loops there is no DerivedDeclarator count to
            // cap later -- the depth has to be refused here.
            let mut ptr_depth = 0u32;
            loop {
                let star_span = self.peek_span();
                if !self.consume_if(&TokenKind::Star) {
                    break;
                }
                ptr_depth += 1;
                if !self.enter_pointer_level(ptr_depth, star_span) {
                    break;
                }
                result_type = TypeSpecifier::Pointer(Box::new(result_type), AddressSpace::Default);
                self.skip_cv_qualifiers();
            }
            // Handle function pointer: type (*)(args)
            if matches!(self.peek(), TokenKind::LParen) {
                let save2 = self.pos;
                self.advance();
                if self.consume_if(&TokenKind::Star) {
                    while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
                        self.advance();
                    }
                    self.consume_if(&TokenKind::RParen);
                    if matches!(self.peek(), TokenKind::LParen) {
                        self.skip_balanced_parens();
                    }
                    result_type =
                        TypeSpecifier::Pointer(Box::new(result_type), AddressSpace::Default);
                } else {
                    self.pos = save2;
                }
            }
            // Parse array dimensions
            while matches!(self.peek(), TokenKind::LBracket) {
                let open = self.peek_span();
                self.advance();
                let size = if matches!(self.peek(), TokenKind::RBracket) {
                    None
                } else {
                    Some(Box::new(self.parse_expr()))
                };
                self.expect_closing(&TokenKind::RBracket, open);
                result_type = TypeSpecifier::Array(Box::new(result_type), size);
            }
            result_type
        } else {
            let span = self.peek_span();
            self.emit_error("expected type in __builtin_va_arg", span);
            TypeSpecifier::Int
        }
    }

    /// Parse a type-name for __builtin_types_compatible_p while retaining the
    /// qualifier level that CType deliberately erases. The returned vector is
    /// ordered base/innermost -> outermost; callers ignore its last element
    /// (top-level CVR qualifiers do not affect type compatibility).
    pub(super) fn parse_compatible_type(&mut self) -> (TypeSpecifier, Vec<u8>) {
        let saved_attrs = self.attrs.clone();
        self.attrs.set_const(false);
        self.attrs.set_volatile(false);
        self.attrs.parsing_address_space = AddressSpace::Default;

        let Some(type_spec) = self.parse_type_specifier() else {
            let span = self.peek_span();
            self.emit_error("expected type in __builtin_types_compatible_p", span);
            self.attrs = saved_attrs;
            return (TypeSpecifier::Int, vec![0]);
        };

        // Capture trailing CVR and address-space qualifiers written after the
        // base specifier (`typeof(x) const *`, `int __seg_gs (*)[4]`).
        let q_start = self.pos;
        self.skip_cv_qualifiers();
        let restrict_bit = u8::from(
            self.tokens[q_start..self.pos]
                .iter()
                .any(|tok| matches!(tok.kind, TokenKind::Restrict)),
        ) << 2;
        let base_quals = u8::from(self.attrs.parsing_const())
            | (u8::from(self.attrs.parsing_volatile()) << 1)
            | restrict_bit;
        let base_space = std::mem::take(&mut self.attrs.parsing_address_space);
        self.attrs.set_const(false);
        self.attrs.set_volatile(false);

        let mut qualifiers = vec![base_quals];
        let mut ptr_budget = 0u32;
        let level = self.parse_abstract_declarator_level(&mut ptr_budget);
        let result_type = level.apply_to(type_spec, base_space, Some(&mut qualifiers));

        self.attrs = saved_attrs;
        (result_type, qualifiers)
    }

    /// Parse a type-name NESTED inside an expression or another type context:
    /// the operand of a cast or compound literal, the argument of `typeof`,
    /// `sizeof`, `_Alignof`/`__alignof__`, `alignas`, `_Generic` associations
    /// and `_Atomic(...)`. A type-name is a specifier-qualifier-list followed
    /// by an optional abstract declarator (C17 6.7.7).
    ///
    /// Address-space scoping — the boundary the __typeof__-steal fix needs:
    /// `attrs.parsing_address_space` is the parser's pending-qualifier slot.
    /// It is SET by a `__seg_gs`/`__seg_fs` token wherever one appears, and
    /// TAKEN (mem::take) by the `*` arms of the declarator parsers. When a
    /// nested type-name is parsed while an ENCLOSING declaration has a
    /// pending qualifier (the kernel's `extern __seg_gs __typeof__(struct
    /// irq_stack *) hardirq_stack_ptr;` — the `__seg_gs` belongs to the
    /// declared object), a `*` inside the nested abstract declarator would
    /// otherwise steal the enclosing qualifier: the typeof type silently
    /// gains the segment, the declaration snapshot reads Default, and every
    /// access to the object compiles as a plain RIP-relative reference to
    /// the static per-CPU image instead of %gs:.
    ///
    /// The nested type-name therefore starts from a CLEAN slot — the
    /// enclosing value is saved, the slot cleared, and the full type-name
    /// (specifier-qualifier-list + abstract declarator, so a qualifier the
    /// nested type-name sets BEFORE its own base type, as in
    /// `(__seg_fs unsigned long *)40`, still reaches its own `*` arm) is
    /// parsed — after which the enclosing value is restored, on every exit
    /// path including failure and caller-side backtracking (the restore
    /// happens before this function returns, so the caller rewinding
    /// `self.pos` cannot resurrect a clobbered slot).
    pub(super) fn parse_nested_type_name(&mut self) -> Option<TypeSpecifier> {
        let enclosing_address_space = self.attrs.parsing_address_space;
        self.attrs.parsing_address_space = AddressSpace::Default;
        // The closure borrows `&mut self` after `parse_type_specifier` has
        // returned (no live borrow of the Option), so the mutable capture
        // is fine; the restore below still runs on both arms of the map.
        let result = self
            .parse_type_specifier()
            .map(|base| self.parse_abstract_declarator_suffix(base));
        self.attrs.parsing_address_space = enclosing_address_space;
        result
    }

    /// Parse an abstract declarator suffix: pointer(s), parenthesized pointer groups,
    /// and array dimensions after a type name. Used by cast expressions, sizeof,
    /// typeof, and _Alignof to avoid duplicating this logic.
    ///
    /// Input: base type already parsed.
    /// Output: type wrapped with pointer/array/function-pointer modifiers.
    ///
    /// Callers that enter this from a nested type-name context (casts,
    /// typeof arguments, ...) must go through `parse_nested_type_name`
    /// instead of calling `parse_type_specifier` + this function directly:
    /// the pending address-space qualifier needs the scoping described
    /// there. Entering with a qualifier set before this suffix's own base
    /// type (e.g. the cast `(__seg_fs unsigned long *)`) is legitimate —
    /// the leading `skip_cv_qualifiers`/`*` arms below capture it.
    pub(super) fn parse_abstract_declarator_suffix(
        &mut self,
        result_type: TypeSpecifier,
    ) -> TypeSpecifier {
        // Consume address space qualifiers that appear before the first '*'
        // e.g., typeof(var) __seg_gs * → __seg_gs sets parsing_address_space
        self.skip_cv_qualifiers();
        let base_space = std::mem::take(&mut self.attrs.parsing_address_space);
        let mut ptr_budget = 0u32;
        let level = self.parse_abstract_declarator_level(&mut ptr_budget);
        if level.is_empty() {
            self.attrs.parsing_address_space = base_space;
            result_type
        } else {
            level.apply_to(result_type, base_space, None)
        }
    }
}

#[cfg(test)]
mod enumerator_attribute_tests {
    use super::super::Parser;
    use crate::frontend::lexer::Lexer;

    #[test]
    fn enumerator_attributes_do_not_create_or_overwrite_constants() {
        for attribute in [
            "__attribute__((deprecated))",
            "[[deprecated]]",
            "__attribute__((deprecated(\"use base\"))) [[maybe_unused]]",
        ] {
            let source =
                format!("enum E {{ base = 4, alias {attribute} = base, next, last = -3 }};");
            let mut parser = Parser::new(Lexer::new(&source, 0).tokenize());
            parser.parse();
            for (name, value) in [("base", 4), ("alias", 4), ("next", 5), ("last", -3)] {
                assert_eq!(parser.enum_constants.get(name), Some(&value), "{source}");
            }
            assert!(!parser.enum_constants.contains_key("deprecated"));
            assert!(!parser.enum_constants.contains_key("maybe_unused"));
        }
    }
}
