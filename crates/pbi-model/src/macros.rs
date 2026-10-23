//! Internal helper macros shared by the model modules.

/// Compares `input` with `candidate`, ignoring ASCII case, surrounding
/// whitespace and the separators ` `, `_` and `-`.
pub(crate) fn lenient_eq(input: &str, candidate: &str) -> bool {
    fn norm(s: &str) -> impl Iterator<Item = char> + '_ {
        s.chars()
            .filter(|c| !matches!(c, ' ' | '_' | '-'))
            .map(|c| c.to_ascii_lowercase())
    }
    norm(input.trim()).eq(norm(candidate))
}

/// Declares a string-valued enum with a canonical spelling per variant, a
/// list of lenient aliases, and an automatic `Unknown(String)` fallback that
/// preserves unrecognised input verbatim.
///
/// Generated API: `as_str`, `parse` (never fails), `is_unknown`, `Display`,
/// `FromStr<Err = Infallible>`, and serde as a plain JSON string.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $canon:literal $(| $alias:literal)*
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant, )+
            /// A value this version does not recognise, preserved verbatim.
            Unknown(::std::string::String),
        }

        impl $name {
            /// Canonical spelling (as written by TMDL/TMSL/PBIR). `Unknown`
            /// values are returned verbatim.
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $canon, )+
                    Self::Unknown(raw) => raw.as_str(),
                }
            }

            /// Lenient parse: case-insensitive, ignores surrounding
            /// whitespace and ` `/`_`/`-` separators, accepts known aliases.
            /// Unrecognised input becomes `Unknown(input)`.
            pub fn parse(input: &str) -> Self {
                $(
                    if $crate::macros::lenient_eq(input, $canon)
                        $( || $crate::macros::lenient_eq(input, $alias) )*
                    {
                        return Self::$variant;
                    }
                )+
                Self::Unknown(input.to_owned())
            }

            /// `true` for the `Unknown` fallback variant.
            pub fn is_unknown(&self) -> bool {
                matches!(self, Self::Unknown(_))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::std::convert::Infallible;
            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                Ok(Self::parse(s))
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let raw = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::parse(&raw))
            }
        }
    };
}

pub(crate) use string_enum;

#[cfg(test)]
mod tests {
    use super::lenient_eq;

    #[test]
    fn lenient_eq_ignores_case_and_separators() {
        assert!(lenient_eq(" Whole Number ", "wholeNumber"));
        assert!(lenient_eq("both_directions", "bothDirections"));
        assert!(!lenient_eq("int", "int64"));
    }
}
