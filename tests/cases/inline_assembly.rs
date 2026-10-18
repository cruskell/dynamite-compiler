use crate::support::{assert_frontend_error, compile_and_run_with_c_flags};
use test_case::test_matrix;

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn register_input_inline_assembly_preserves_operands((output_option, optimization): (&str, &str)) {
    let (wide, narrow, pointer, update) = if cfg!(target_arch = "aarch64") {
        (
            "add %x0, %x1, %x2",
            "add %w0, %w1, %w2",
            "mov %0, %1",
            "add %0, %0, %1",
        )
    } else {
        (
            "lea (%q1,%q2), %q0",
            "lea (%k1,%k2), %k0",
            "mov %1, %0",
            "add %1, %0",
        )
    };
    let code = format!(
        r#"
int calls;unsigned long left(void){{calls++;return 5;}}unsigned long right(void){{calls++;return 7;}}
volatile long source=4;
int main(){{
 unsigned long sum;asm("{wide}" : "=r"(sum) : "r"(left()),"r"(right()));if(sum!=12 || calls!=2)return 1;
 int signed_sum;asm("{narrow}" : "=r"(signed_sum) : "r"(-9),"r"(5));if(signed_sum!=-4)return 2;
 char array[4];void *pointer;asm("{pointer}" : "=r"(pointer) : "r"(array));if(pointer!=(void*)array)return 3;
 long value=3;asm volatile("{update}" : "+r"(value) : "r"(source));if(value!=7)return 4;
 long guard=17;asm("" : "+r"(guard) : "r"(1),"r"(2),"r"(3),"r"(4),"r"(5),"r"(6));if(guard!=17)return 5;
 return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[cfg(target_arch = "aarch64")]
#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn register_input_inline_assembly_supports_aarch64_width_modifiers(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
int calls;unsigned int left(void){calls++;return 4294967295U;}unsigned int right(void){calls++;return 2;}
unsigned long accumulator(void){calls++;return 7;}
int main(){unsigned long value;asm("umaddl %x0, %w1, %w2, %x3" : "=r"(value) : "r"(left()),"r"(right()),"r"(accumulator()));return value!=8589934597UL || calls!=3;}
"#;
    assert_eq!(
        compile_and_run_with_c_flags(code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn register_input_inline_assembly_retains_explicit_operand_numbering() {
    let code = "volatile int output=7,input=11;int main(){asm volatile(\"\" : \"+r\"(output) : \"r\"(input));return 0;}";
    let generated = dynamite_compiler::generate(code, "llvm", false);
    assert!(generated.contains("=r,r,0,"), "{generated}");
    assert_eq!(
        generated.matches("load volatile i32").count(),
        2,
        "{generated}"
    );
}

#[test_matrix([
    "int main(){int x;asm(\"%2\" : \"=r\"(x) : \"r\"(1));}",
    "int main(){int x;asm(\"%99999999999999999999999999\" : \"=r\"(x));}",
    "int main(){int x;asm(\"nop\" : \"=r\"(x) : \"r\"(1.0));}",
    "int main(){int x;char y=1;asm(\"nop\" : \"=r\"(x) : \"r\"(y));}",
    "int main(){int x;asm(\"nop\" : \"=r\"(x) : \"r\"(1),\"r\"(2),\"r\"(3),\"r\"(4),\"r\"(5),\"r\"(6),\"r\"(7));}",
], ["asm", "llvm"])]
fn invalid_register_input_inline_assembly_has_source_diagnostics(code: &str, output_option: &str) {
    let panic =
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .unwrap_err();
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains('^'), "{message}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn readwrite_inline_assembly_evaluates_destinations_once(
    (output_option, optimization): (&str, &str),
) {
    let template = if cfg!(target_arch = "aarch64") {
        "add %0, %0, #5"
    } else {
        "add $5, %0"
    };
    let code = format!(
        r#"
volatile int global=12;int calls;volatile int *destination(void){{calls++;return &global;}}
int main(){{
 asm volatile("{template}" : "+r"(*destination()));if(calls!=1 || global!=17)return 1;
 int signed_value=-9;asm("{template}" : "+r"(signed_value));if(signed_value!=-4)return 2;
 unsigned int wrapped=4294967293U;asm("{template}" : "+r"(wrapped));if(wrapped!=2)return 3;
 unsigned long wide=4294967296UL;asm("{template}" : "+r"(wide));if(wide!=4294967301UL)return 4;
 void *pointer=(void*)37;asm("" : "+r"(pointer));if(pointer!=(void*)37)return 5;
 typedef int U __attribute__((aligned(1)));unsigned char bytes[8];U *unaligned=(U*)(bytes+1);*unaligned=21;
 asm("{template}" : "+r"(*unaligned));if(*unaligned!=26)return 6;
 return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn readwrite_inline_assembly_ties_input_and_output_in_llvm() {
    let generated = dynamite_compiler::generate(
        "volatile int output=7;int main(){asm volatile(\"\" : \"+r\"(output));return output!=7;}",
        "llvm",
        false,
    );
    assert!(generated.contains("=r,0,"), "{generated}");
    assert!(generated.contains("load volatile i32"), "{generated}");
    assert!(generated.contains("store volatile i32"), "{generated}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn register_output_inline_assembly_preserves_values_and_lvalues(
    (output_option, optimization): (&str, &str),
) {
    let (positive, negative, zero) = if cfg!(target_arch = "aarch64") {
        (
            "mov %0, #42\nmov %0, %0 // %% literal",
            "mov %0, #-7",
            "mov %0, #0",
        )
    } else {
        (
            "mov $42, %0\nmov %0, %0 # %% literal",
            "mov $-7, %0",
            "mov $0, %0",
        )
    };
    let positive = positive.replace('\n', "\\n");
    let code = format!(
        r#"
int calls;volatile int global;int *destination(void){{calls++;return (int*)&global;}}
long signed_value(void){{int n;__asm__("{negative}" : "=r"(n));return n;}}
unsigned long unsigned_value(void){{unsigned int n;__asm__ __volatile__("{negative}" : "=r"(n));return n;}}
#pragma pack(push,1)
struct Packed{{char tag;long value;}};
#pragma pack(pop)
int main(void){{
 long a=3,b=5,value;asm volatile("{positive}" : "=r"(value));
 if(a+b+value!=50)return 1;
 __asm volatile("{positive}" : "=r"(*destination()) : :);
 if(calls!=1 || global!=42)return 2;
 if(signed_value()!=-7 || unsigned_value()!=4294967289UL)return 3;
 void *pointer=(void*)1;__asm__("{zero}" : "=r"(pointer));if(pointer)return 4;
 struct Packed packed;asm("{positive}" : "=r"(packed.value));if(packed.value!=42)return 5;
 int n=2,index=0;int array[n];__asm__("{positive}" : "=r"(array[index++]));if(index!=1 || array[0]!=42)return 6;
 return 0;
}}
"#
    );
    assert_eq!(
        compile_and_run_with_c_flags(&code, output_option, None, &[optimization])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn register_output_inline_assembly_preserves_volatile_ir() {
    let template = if cfg!(target_arch = "aarch64") {
        "mov %0, #42"
    } else {
        "mov $42, %0"
    };
    for qualifier in ["volatile", ""] {
        let code = format!(
            "volatile int output;int main(){{asm {qualifier}(\"{template}\" : \"=r\"(output));return output!=42;}}"
        );
        let generated = dynamite_compiler::generate(&code, "llvm", false);
        assert_eq!(
            generated.contains("asm sideeffect"),
            !qualifier.is_empty(),
            "{generated}"
        );
        assert!(generated.contains("store volatile i32"), "{generated}");
    }
}

#[test_matrix([
    "int main(){int x;asm(\"nop\");}",
    "int main(){int x;asm goto(\"nop\");}",
    "int main(){int x;asm(\"%\" : \"=r\"(x));}",
    "int main(){int x;asm(\"%1\" : \"=r\"(x));}",
    "int main(){int x;asm(\"%Z0\" : \"=r\"(x));}",
    "int main(){int x;asm(\"%[name]\" : \"=r\"(x));}",
    "int main(){int memory;asm(\"nop\" : \"+m\"(memory));}",
    "int main(){int x;asm(\"nop\" : \"=m\"(x));}",
    "int main(){int x;asm(\"nop\" : \"=r\"(x) : \"m\"(1));}",
    "int main(){asm(\"\" : : : \"unsupported_register\");}",
    "int main(){const int x=0;asm(\"nop\" : \"=r\"(x));}",
    "int main(){asm(\"nop\" : \"=r\"(1));}",
    "int main(){double x;asm(\"nop\" : \"=r\"(x));}",
    "int main(){char x;asm(\"nop\" : \"=r\"(x));}",
    "struct S{int n;};int main(){struct S x;asm(\"nop\" : \"=r\"(x));}",
    "struct S{int n:3;};int main(){struct S x;asm(\"nop\" : \"=r\"(x.n));}",
    "int main(){int x;asm(L\"nop\" : \"=r\"(x));}",
    "int main(){int x;asm(\"nop\\0bad\" : \"=r\"(x));}",
], ["asm", "llvm"])]
fn unsupported_inline_assembly_has_source_diagnostics(code: &str, output_option: &str) {
    let panic =
        std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
            .unwrap_err();
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains('^'), "{message}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn multiple_asm_outputs_preserve_values_and_lvalue_effects(
    (output_option, optimization): (&str, &str),
) {
    let code = r#"
int calls;int value=11;int *get_pointer(void){calls++;return &value;}
int main(void){volatile int values[2]={3,5};int i=0;unsigned long wide=7;
    asm volatile("" : "+r"(values[i++]), "+r"(*get_pointer()), "+r"(wide));
    long a=1,b=2,c=3,d=4,e=5,f=6,g=7;
    asm("" : "+r"(a), "+r"(b), "+r"(c), "+r"(d), "+r"(e), "+r"(f), "+r"(g));
    return i!=1 || calls!=1 || values[0]!=3 || value!=11 || wide!=7
        || a!=1 || b!=2 || c!=3 || d!=4 || e!=5 || f!=6 || g!=7;}
"#;
    let output = compile_and_run_with_c_flags(code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn multiple_asm_outputs_preserve_types_and_operand_numbers(
    (output_option, optimization): (&str, &str),
) {
    let arithmetic = if cfg!(target_arch = "aarch64") {
        r"add %w0, %w0, %w3\n\tadd %x1, %x1, %x4\n\tmov %w2, #17"
    } else {
        r"addl %3, %0\n\taddq %4, %1\n\tmovl $17, %2"
    };
    let pointer = if cfg!(target_arch = "aarch64") {
        r"mov %x1, %x2\n\tmov %w0, #19"
    } else {
        r"movq %2, %1\n\tmovl $19, %0"
    };
    let boundary = if cfg!(target_arch = "aarch64") {
        "add %w0, %w0, %w2"
    } else {
        "addl %2, %0"
    };
    let code = format!(
        r#"
int object;
int main(void){{int a=-4;unsigned long b=9;volatile int c;int x=5;unsigned long y=13;
    asm volatile("{arithmetic}" : "+r"(a), "+r"(b), "=r"(c) : "r"(x), "r"(y));
    if(a!=1 || b!=22 || c!=17)return 1;
    void *address;int tag;
    asm("{pointer}" : "=r"(tag), "=r"(address) : "r"(&object));
    if(address!=&object || tag!=19)return 2;
    int first=7,second=11;
    asm("{boundary}" : "+r"(first), "+r"(second) : "r"(1), "r"(2), "r"(3), "r"(4), "r"(5));
    return first!=8 || second!=11;}}
"#
    );
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test_matrix([
    "int main(){int x=1;asm(\"\" : \"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x),\"+r\"(x));}",
    "int main(){int x=1,y=2;asm(\"\" : \"+r\"(x),\"+r\"(y) : \"r\"(1),\"r\"(2),\"r\"(3),\"r\"(4),\"r\"(5),\"r\"(6));}",
    "int main(){int x,y;asm(\"%2\" : \"=r\"(x),\"=r\"(y));}",
    "int main(){int x,y;asm(\"\" : \"=r\"(x),\"=&r\"(y));}",
])]
fn unsupported_multiple_asm_outputs_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test_matrix([("asm", "-O0"), ("llvm", "-O0"), ("llvm", "-O2")])]
fn memory_clobber_inline_assembly_preserves_memory_effects(
    (output_option, optimization): (&str, &str),
) {
    let store = if cfg!(target_arch = "aarch64") {
        "str %w1, [%0]"
    } else {
        "movl %1, (%0)"
    };
    let code = format!(
        r#"
int calls;int value(void){{calls++;return 19;}}
int read(void){{int local=3;
    asm("{store}" : : "r"(&local), "r"(value()) : "memory");
    return local;
}}
int main(void){{
    asm volatile("" ::: "memory");
    __asm__ __volatile__("" : : : "memory", "memory");
    asm("nop" :);asm("" : :);asm("" : : :);
    long same=7;asm("" : "+r"(same) : : "memory");
    int result=read();int expression=5+({{asm("" ::: "memory");3;}});
    return result!=19 || calls!=1 || same!=7 || expression!=8;
}}
"#
    );
    let output = compile_and_run_with_c_flags(&code, output_option, None, &[optimization]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn memory_clobber_inline_assembly_emits_llvm_barriers() {
    let generated = dynamite_compiler::generate(
        "int value;void barrier(void){value=3;asm(\"\" ::: \"memory\");value=7;}",
        "llvm",
        false,
    );
    assert!(
        generated.contains("call void asm sideeffect"),
        "{generated}"
    );
    assert!(generated.contains("~{memory}"), "{generated}");
    assert!(!generated.contains("call {  } asm"), "{generated}");
}
