use crate::support::{assert_frontend_error, compile_and_run, compile_and_run_with_c};
use test_case::test_matrix;

#[test_matrix([
    "int f(int n,int m,int a[n][m]){return a[n-1][m-1]+sizeof(*a);}int main(){int n=2;int m=3;int a[n][m];a[1][2]=7;return f(n,m,a)-19;}",
    "int main(){int n=2;int m=3;int a[n][m];int (*p)[m]=a;p++;p[0][2]=7;return a[1][2]+(p-a)-8;}",
    "int main(){int n=3;int a[n][2];a[2][1]=5;return a[2][1]+sizeof(a)-29;}",
    "int main(){int n=2;int m=3;int a[n][m];int (*p)[m]=a;int (*q)[m]=p++;return (p-q)-1;}",
], ["asm", "llvm"])]
fn it_supports_variable_array_parameters_and_pointers(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn it_releases_variable_arrays_on_scope_exits(output_option: &str) {
    let code = "char pool[10000];unsigned long top;int live;void *malloc(unsigned long n){live++;void *p=pool+top;top+=n;return p;}void free(void *p){live--;}int f(int n){int a[n];a[0]=9;return a[0];}int main(){live=0;top=0;int n=3;typedef int A[n];{A a;goto out;}out:if(live)return 1;for(int i=0;i<4;i++){int a[n];if(i==1)continue;if(i==2)break;}if(live)return 2;int s=f(n);if(live)return 3;return s-9;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int n;int a[n];",
    "int main(){int n=2;static int a[n];}",
    "int main(){int n=2;int a[n]={1};}",
    "int main(){int n=2;goto label;int a[n];label:return 0;}",
    "int main(){int n=2;struct S{int a[n];};return 0;}",
    "int main(){int n=2;switch(n){int a[n];case 2:return 0;}return 1;}",
])]
fn it_diagnoses_invalid_variable_arrays(code: &str) {
    assert!(std::panic::catch_unwind(|| dynamite_compiler::generate(code, "asm", false)).is_err());
}

#[test_matrix([
    "int main(){int n=3;int a[n];void *p=a;int (*q)[n]=(int (*)[n])p;(*q)[2]=7;return a[2]-7;}",
    "int main(){int n=3;goto label;int s=sizeof(int[n]);label:return 0;}",
    "int main(){int n=3;const int a[n];a[1]=2;return 0;}",
], ["asm", "llvm"])]
fn it_checks_variable_array_expression_scopes(code: &str, output_option: &str) {
    if code.contains("const int") {
        assert!(
            std::panic::catch_unwind(|| dynamite_compiler::generate(code, output_option, false))
                .is_err()
        );
    } else {
        assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
    }
}

#[test_matrix([
    "int main(){int n=2;int m=3;int a[n][(int)m];a[1][2]=7;return sizeof(a)+a[1][2]-31;}",
    "int f(int n,int a[n][n]);int f(int n,int a[n][n]){return a[n-1][n-1];}int main(){int n=2;int a[n][n];a[1][1]=7;return f(n,a)-7;}",
    "int main(){int n=2;int a[n][n];static int (*p)[n];p=a;p[1][1]=7;return a[1][1]-7;}",
], ["asm", "llvm"])]
fn it_preserves_variable_array_bound_evaluation(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "int main(){int n=3;int i=0;for(int a[n];i<2;i++){a[0]=i;}return i-2;}",
    "int main(){int n=3;int x=0;again:{int a[n];a[0]=1;x+=a[0];if(x<3)goto again;}return x-3;}",
    "int a[(int)3.0];int main(){return sizeof(a)-12;}",
], ["asm", "llvm"])]
fn it_supports_variable_array_loop_lifetimes(code: &str, output_option: &str) {
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix([
    "void *p=__builtin_alloca(4);",
    "int main(){__builtin_alloca(1.5);return 0;}",
    "int main(){int x;__builtin_alloca(&x);return 0;}",
    "int main(){__builtin_alloca(4,8);return 0;}",
])]
fn invalid_alloca_calls_are_diagnosed(code: &str) {
    assert_frontend_error(code);
}

#[test]
fn native_alloca_has_a_source_diagnostic() {
    let result = std::panic::catch_unwind(|| {
        dynamite_compiler::generate(
            "int main(){void *p=__builtin_alloca(8);return p==0;}",
            "asm",
            false,
        )
    });
    let error = result.unwrap_err();
    let message = error
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains("alloca requires the LLVM backend") && message.contains('^'));
}

#[test_matrix(["asm", "llvm"])]
fn register_vla_size(output_option: &str) {
    let code = "int main(){int n=3;register int a[n];return sizeof(a)-12;}";
    assert_eq!(compile_and_run(code, output_option).status.code(), Some(0));
}

#[test_matrix(["asm", "llvm"])]
fn incomplete_extern_arrays_link_without_allocating_storage(output_option: &str) {
    let code = "extern int values[];int *value_tail=values+2;int read_values(){extern int values[];return values[0]+*value_tail;}int main(){return read_values()-9;}";
    let companion = "int values[3]={2,3,7};";
    assert_eq!(
        compile_and_run_with_c(code, output_option, Some(companion))
            .status
            .code(),
        Some(0)
    );
}
