//! Predefined macros for Dynamite's supported native C data models.

use super::Preprocessor;

pub(super) fn define(pp: &mut Preprocessor) -> Result<(), String> {
    for (name, value) in [
        ("__DYNAMITE__", "1"),
        ("__STDC__", "1"),
        ("__STDC_HOSTED__", "1"),
        ("__STDC_VERSION__", "199901L"),
        ("__GNUC__", "4"),
        ("__GNUC_MINOR__", "2"),
        ("__GNUC_PATCHLEVEL__", "1"),
        ("__GNUC_STDC_INLINE__", "1"),
        ("__LP64__", "1"),
        ("_LP64", "1"),
        ("__CHAR_BIT__", "8"),
        ("__SCHAR_MAX__", "127"),
        ("__SHRT_MAX__", "32767"),
        ("__INT_MAX__", "2147483647"),
        ("__LONG_MAX__", "9223372036854775807L"),
        ("__LONG_LONG_MAX__", "9223372036854775807LL"),
        ("__SIZE_TYPE__", "long unsigned int"),
        ("__PTRDIFF_TYPE__", "long int"),
        ("__WCHAR_TYPE__", "int"),
        ("__WINT_TYPE__", "unsigned int"),
        ("__WCHAR_MAX__", "2147483647"),
        ("__WINT_MAX__", "4294967295U"),
        ("__WINT_UNSIGNED__", "1"),
        ("__SIZE_MAX__", "18446744073709551615UL"),
        ("__PTRDIFF_MAX__", "9223372036854775807L"),
        ("__BIGGEST_ALIGNMENT__", "16"),
        ("__ORDER_LITTLE_ENDIAN__", "1234"),
        ("__ORDER_BIG_ENDIAN__", "4321"),
        ("__ORDER_PDP_ENDIAN__", "3412"),
        ("__BYTE_ORDER__", "__ORDER_LITTLE_ENDIAN__"),
        ("__LITTLE_ENDIAN__", "1"),
        ("__NO_INLINE__", "1"),
        ("__NO_MATH_INLINES", "1"),
        ("__FINITE_MATH_ONLY__", "0"),
        ("__GCC_ASM_FLAG_OUTPUTS__", "1"),
        ("__GCC_HAVE_DWARF2_CFI_ASM", "1"),
        ("__GCC_ATOMIC_TEST_AND_SET_TRUEVAL", "1"),
        ("__PIC__", "2"),
        ("__pic__", "2"),
        ("__PIE__", "2"),
        ("__pie__", "2"),
        ("__USER_LABEL_PREFIX__", ""),
        ("__REGISTER_PREFIX__", ""),
        ("__VERSION__", "\"Dynamite\""),
        ("__FLT_RADIX__", "2"),
        ("__DECIMAL_DIG__", "__LDBL_DECIMAL_DIG__"),
        ("__FLT_MANT_DIG__", "24"),
        ("__FLT_DIG__", "6"),
        ("__FLT_DECIMAL_DIG__", "9"),
        ("__FLT_MIN_EXP__", "(-125)"),
        ("__FLT_MAX_EXP__", "128"),
        ("__FLT_MIN_10_EXP__", "(-37)"),
        ("__FLT_MAX_10_EXP__", "38"),
        ("__FLT_MIN__", "1.17549435e-38F"),
        ("__FLT_MAX__", "3.40282347e+38F"),
        ("__FLT_EPSILON__", "1.19209290e-7F"),
        ("__FLT_DENORM_MIN__", "1.40129846e-45F"),
        ("__DBL_MANT_DIG__", "53"),
        ("__DBL_DIG__", "15"),
        ("__DBL_DECIMAL_DIG__", "17"),
        ("__DBL_MIN_EXP__", "(-1021)"),
        ("__DBL_MAX_EXP__", "1024"),
        ("__DBL_MIN_10_EXP__", "(-307)"),
        ("__DBL_MAX_10_EXP__", "308"),
        ("__DBL_MIN__", "2.2250738585072014e-308"),
        ("__DBL_MAX__", "1.7976931348623157e+308"),
        ("__DBL_EPSILON__", "2.2204460492503131e-16"),
        ("__DBL_DENORM_MIN__", "4.9406564584124654e-324"),
        ("__LDBL_MANT_DIG__", "64"),
        ("__LDBL_DIG__", "18"),
        ("__LDBL_DECIMAL_DIG__", "21"),
        ("__LDBL_MIN_EXP__", "(-16381)"),
        ("__LDBL_MAX_EXP__", "16384"),
        ("__LDBL_MIN_10_EXP__", "(-4931)"),
        ("__LDBL_MAX_10_EXP__", "4932"),
        ("__LDBL_MIN__", "3.36210314311209350626e-4932L"),
        ("__LDBL_MAX__", "1.18973149535723176502e+4932L"),
        ("__LDBL_EPSILON__", "1.08420217248550443401e-19L"),
        ("__LDBL_DENORM_MIN__", "3.64519953188247460253e-4951L"),
    ] {
        pp.define(&format!("{name} {value}"))?;
    }
    for (name, value) in [
        ("SHORT", 2),
        ("INT", 4),
        ("LONG", 8),
        ("LONG_LONG", 8),
        ("POINTER", 8),
        ("FLOAT", 4),
        ("DOUBLE", 8),
        ("LONG_DOUBLE", 16),
        ("SIZE_T", 8),
        ("PTRDIFF_T", 8),
        ("WCHAR_T", 4),
        ("WINT_T", 4),
    ] {
        pp.define(&format!("__SIZEOF_{name}__ {value}"))?;
    }
    for prefix in ["FLT", "DBL", "LDBL"] {
        for suffix in ["HAS_DENORM", "HAS_INFINITY", "HAS_QUIET_NAN"] {
            pp.define(&format!("__{prefix}_{suffix}__ 1"))?;
        }
    }
    for (name, value) in [
        ("RELAXED", 0),
        ("CONSUME", 1),
        ("ACQUIRE", 2),
        ("RELEASE", 3),
        ("ACQ_REL", 4),
        ("SEQ_CST", 5),
    ] {
        pp.define(&format!("__ATOMIC_{name} {value}"))?;
    }
    for name in [
        "BOOL", "CHAR", "CHAR16_T", "CHAR32_T", "WCHAR_T", "SHORT", "INT", "LONG", "LLONG",
        "POINTER",
    ] {
        pp.define(&format!("__GCC_ATOMIC_{name}_LOCK_FREE 2"))?;
    }
    for size in [1, 2, 4, 8] {
        pp.define(&format!("__GCC_HAVE_SYNC_COMPARE_AND_SWAP_{size} 1"))?;
    }
    for (bits, ty, max, suffix, fmt) in [
        (8, "signed char", "127", "", "hh"),
        (16, "short", "32767", "", "h"),
        (32, "int", "2147483647", "", ""),
        (64, "long int", "9223372036854775807L", "L", "l"),
    ] {
        for prefix in [
            format!("INT{bits}"),
            format!("INT_LEAST{bits}"),
            format!("INT_FAST{bits}"),
        ] {
            for (property, value) in [
                ("TYPE", ty.to_string()),
                ("MAX", max.to_string()),
                ("WIDTH", bits.to_string()),
                ("C_SUFFIX", suffix.to_string()),
                ("FMTd", format!("\"{fmt}d\"")),
                ("FMTi", format!("\"{fmt}i\"")),
            ] {
                pp.define(&format!("__{prefix}_{property}__ {value}"))?;
            }
        }
        let umax = match bits {
            8 => "255",
            16 => "65535",
            32 => "4294967295U",
            _ => "18446744073709551615UL",
        };
        for prefix in [
            format!("UINT{bits}"),
            format!("UINT_LEAST{bits}"),
            format!("UINT_FAST{bits}"),
        ] {
            let ty = ty.replace("signed ", "");
            for (property, value) in [
                ("TYPE", format!("unsigned {ty}")),
                ("MAX", umax.to_string()),
                ("WIDTH", bits.to_string()),
                (
                    "C_SUFFIX",
                    if bits == 64 {
                        "UL"
                    } else if bits == 32 {
                        "U"
                    } else {
                        ""
                    }
                    .to_string(),
                ),
                ("FMTu", format!("\"{fmt}u\"")),
                ("FMTx", format!("\"{fmt}x\"")),
                ("FMTX", format!("\"{fmt}X\"")),
                ("FMTo", format!("\"{fmt}o\"")),
            ] {
                pp.define(&format!("__{prefix}_{property}__ {value}"))?;
            }
        }
    }
    for prefix in ["INTMAX", "INTPTR"] {
        for (property, value) in [
            ("TYPE", "long int"),
            ("MAX", "9223372036854775807L"),
            ("WIDTH", "64"),
            ("C_SUFFIX", "L"),
            ("FMTd", "\"ld\""),
            ("FMTi", "\"li\""),
        ] {
            pp.define(&format!("__{prefix}_{property}__ {value}"))?;
        }
    }
    for prefix in ["UINTMAX", "UINTPTR"] {
        for (property, value) in [
            ("TYPE", "long unsigned int"),
            ("MAX", "18446744073709551615UL"),
            ("WIDTH", "64"),
            ("C_SUFFIX", "UL"),
            ("FMTu", "\"lu\""),
            ("FMTx", "\"lx\""),
            ("FMTX", "\"lX\""),
            ("FMTo", "\"lo\""),
        ] {
            pp.define(&format!("__{prefix}_{property}__ {value}"))?;
        }
    }
    if cfg!(target_arch = "x86_64") {
        for name in ["__x86_64__", "__x86_64", "__amd64__", "__amd64"] {
            pp.define(&format!("{name} 1"))?;
        }
    }
    if cfg!(target_os = "linux") {
        for name in [
            "__linux__",
            "__linux",
            "linux",
            "__gnu_linux__",
            "__unix__",
            "__unix",
            "unix",
            "__ELF__",
        ] {
            pp.define(&format!("{name} 1"))?;
        }
    }
    if cfg!(target_os = "macos") {
        pp.undefine("__WINT_UNSIGNED__");
        for (name, value) in [
            ("__APPLE__", "1"),
            ("__APPLE_CC__", "6000"),
            ("__MACH__", "1"),
            ("__USER_LABEL_PREFIX__", "_"),
            ("__WINT_TYPE__", "int"),
            ("__WINT_MAX__", "2147483647"),
        ] {
            pp.define(&format!("{name} {value}"))?;
        }
        let version =
            crate::toolchain::macos_version_number(&crate::toolchain::macos_deployment_target()?)?;
        pp.define(&format!(
            "__ENVIRONMENT_MAC_OS_X_VERSION_MIN_REQUIRED__ {version}"
        ))?;
        pp.define(&format!(
            "__ENVIRONMENT_OS_VERSION_MIN_REQUIRED__ {version}"
        ))?;
        for prefix in ["INT64", "INT_LEAST64", "INT_FAST64"] {
            for (property, value) in [
                ("TYPE", "long long int"),
                ("MAX", "9223372036854775807LL"),
                ("C_SUFFIX", "LL"),
                ("FMTd", "\"lld\""),
                ("FMTi", "\"lli\""),
            ] {
                pp.define(&format!("__{prefix}_{property}__ {value}"))?;
            }
        }
        for prefix in ["UINT64", "UINT_LEAST64", "UINT_FAST64"] {
            for (property, value) in [
                ("TYPE", "long long unsigned int"),
                ("MAX", "18446744073709551615ULL"),
                ("C_SUFFIX", "ULL"),
                ("FMTu", "\"llu\""),
                ("FMTx", "\"llx\""),
                ("FMTX", "\"llX\""),
                ("FMTo", "\"llo\""),
            ] {
                pp.define(&format!("__{prefix}_{property}__ {value}"))?;
            }
        }
    }
    if cfg!(target_arch = "aarch64") {
        for (name, value) in [
            ("__aarch64__", "1"),
            ("__ARM_ARCH", "8"),
            ("__ARM_ARCH_ISA_A64", "1"),
            ("__ARM_64BIT_STATE", "1"),
            ("__ARM_PCS_AAPCS64", "1"),
            ("__ARM_SIZEOF_MINIMAL_ENUM", "4"),
            ("__ARM_SIZEOF_WCHAR_T", "4"),
        ] {
            pp.define(&format!("{name} {value}"))?;
        }
    }
    if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        for (name, value) in [
            ("__arm64__", "1"),
            ("__arm64", "1"),
            ("__ARM64_ARCH_8__", "1"),
            ("__SIZEOF_LONG_DOUBLE__", "8"),
            ("__LDBL_MANT_DIG__", "53"),
            ("__LDBL_DIG__", "15"),
            ("__LDBL_DECIMAL_DIG__", "17"),
            ("__LDBL_MIN_EXP__", "(-1021)"),
            ("__LDBL_MAX_EXP__", "1024"),
            ("__LDBL_MIN_10_EXP__", "(-307)"),
            ("__LDBL_MAX_10_EXP__", "308"),
            ("__LDBL_MIN__", "2.2250738585072014e-308L"),
            ("__LDBL_MAX__", "1.7976931348623157e+308L"),
            ("__LDBL_EPSILON__", "2.2204460492503131e-16L"),
            ("__LDBL_DENORM_MIN__", "4.9406564584124654e-324L"),
        ] {
            pp.define(&format!("{name} {value}"))?;
        }
    }
    if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
        for (name, value) in [
            ("__AARCH64EL__", "1"),
            ("__WCHAR_UNSIGNED__", "1"),
            ("__WCHAR_TYPE__", "unsigned int"),
            ("__WCHAR_MAX__", "4294967295U"),
            ("__SIZEOF_LONG_DOUBLE__", "16"),
            ("__LDBL_MANT_DIG__", "113"),
            ("__LDBL_DIG__", "33"),
            ("__LDBL_DECIMAL_DIG__", "36"),
            ("__LDBL_MIN_EXP__", "(-16381)"),
            ("__LDBL_MAX_EXP__", "16384"),
            ("__LDBL_MIN_10_EXP__", "(-4931)"),
            ("__LDBL_MAX_10_EXP__", "4932"),
            (
                "__LDBL_MIN__",
                "3.36210314311209350626267781732175260e-4932L",
            ),
            (
                "__LDBL_MAX__",
                "1.18973149535723176508575932662800702e+4932L",
            ),
            (
                "__LDBL_EPSILON__",
                "1.92592994438723585305597794258492732e-34L",
            ),
            (
                "__LDBL_DENORM_MIN__",
                "6.47517511943802511092443895822764655e-4966L",
            ),
        ] {
            pp.define(&format!("{name} {value}"))?;
        }
    }
    Ok(())
}
