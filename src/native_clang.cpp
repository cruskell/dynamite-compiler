// Clang 15 handles preprocessing and LLVM IR only; C parsing stays in Dynamite.
#include <clang/Basic/Diagnostic.h>
#include <clang/Basic/DiagnosticOptions.h>
#include <clang/Driver/Compilation.h>
#include <clang/Driver/Driver.h>
#include <clang/Driver/Job.h>
#include <clang/Frontend/CompilerInstance.h>
#include <clang/Frontend/CompilerInvocation.h>
#include <clang/Frontend/FrontendAction.h>
#include <clang/Frontend/Utils.h>
#include <clang/FrontendTool/Utils.h>
#include <llvm/Support/MemoryBuffer.h>
#include <llvm/Support/TargetSelect.h>
#include <mutex>
#include <clang/Frontend/TextDiagnosticPrinter.h>
#include <llvm/ADT/SmallVector.h>
#include <llvm/ADT/SmallString.h>
#include <llvm/Support/FileSystem.h>
#include <llvm/Support/Host.h>
#include <llvm/Support/Program.h>
#include <llvm/Support/raw_ostream.h>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>

class BufferPreprocessAction : public clang::PreprocessorFrontendAction {
  llvm::SmallVectorImpl<char> &buffer;
  void ExecuteAction() override {
    llvm::raw_svector_ostream stream(buffer);
    auto &compiler = getCompilerInstance();
    clang::DoPrintPreprocessedInput(compiler.getPreprocessor(), &stream,
                                   compiler.getPreprocessorOutputOpts());
  }
public:
  explicit BufferPreprocessAction(llvm::SmallVectorImpl<char> &value) : buffer(value) {}
};

static void copy_buffer(llvm::StringRef value, char **data, size_t *size) {
  *size = value.size();
  *data = static_cast<char *>(std::malloc(value.size() + 1));
  if (!*data) std::abort();
  std::memcpy(*data, value.data(), value.size());
  (*data)[value.size()] = 0;
}
extern "C" void dynamite_pp_free(char *data) { std::free(data); }
static int invoke(int argc, const char *const *argv, const char *ir, size_t ir_size,
                            char **output, size_t *output_size,
                            char **diagnostics, size_t *diagnostics_size) {
  std::string diagnostic_text;
  llvm::raw_string_ostream diagnostic_stream(diagnostic_text);
  auto diagnostic_options = llvm::makeIntrusiveRefCnt<clang::DiagnosticOptions>();
  auto diagnostic_ids = llvm::makeIntrusiveRefCnt<clang::DiagnosticIDs>();
  clang::TextDiagnosticPrinter printer(diagnostic_stream, diagnostic_options.get());
  clang::DiagnosticsEngine engine(diagnostic_ids, diagnostic_options, &printer, false);
  auto executable = llvm::sys::findProgramByName(argv[0]);
  std::string path = executable ? *executable : argv[0];
  llvm::SmallString<256> resolved;
  if (!llvm::sys::fs::real_path(path, resolved)) path = resolved.str().str();
  clang::driver::Driver driver(path, llvm::sys::getDefaultTargetTriple(), engine);
  if (ir) driver.setCheckInputsExist(false);
  std::unique_ptr<clang::driver::Compilation> compilation(driver.BuildCompilation({argv, static_cast<size_t>(argc)}));
  bool success = false;
  llvm::SmallVector<char, 0> expanded;
  if (compilation && !engine.hasErrorOccurred() && compilation->getJobs().size() == 1) {
    const auto &arguments = compilation->getJobs().begin()->getArguments();
    auto invocation = std::make_shared<clang::CompilerInvocation>();
    if (!arguments.empty() && std::strcmp(arguments[0], "-cc1") == 0 &&
        clang::CompilerInvocation::CreateFromArgs(*invocation, llvm::ArrayRef<const char *>(arguments).drop_front(), engine, path.c_str())) {
      std::unique_ptr<llvm::MemoryBuffer> ir_buffer;
      clang::CompilerInstance compiler;
      invocation->getFrontendOpts().DisableFree = false;
      compiler.setInvocation(invocation);
      compiler.createDiagnostics(&printer, false);
      if (ir) {
        auto &inputs = invocation->getFrontendOpts().Inputs;
        if (inputs.size() == 1 && inputs.front().getKind().getLanguage() == clang::Language::LLVM_IR) {
          static std::once_flag initialized;
          std::call_once(initialized, [] {
            if (llvm::InitializeNativeTarget() || llvm::InitializeNativeTargetAsmPrinter() ||
                llvm::InitializeNativeTargetAsmParser()) std::abort();
          });
          ir_buffer = llvm::MemoryBuffer::getMemBufferCopy(llvm::StringRef(ir, ir_size), "dynamite.ll");
          inputs.front() = clang::FrontendInputFile(ir_buffer->getMemBufferRef(), clang::InputKind(clang::Language::LLVM_IR));
          success = clang::ExecuteCompilerInvocation(&compiler);
        } else {
          diagnostic_stream << "in-process lowering requires LLVM IR input\n";
        }
      } else {
        BufferPreprocessAction action(expanded);
        success = compiler.ExecuteAction(action);
      }
    }
  }
  diagnostic_stream.flush();
  copy_buffer(llvm::StringRef(expanded.data(), expanded.size()), output, output_size);
  copy_buffer(diagnostic_text, diagnostics, diagnostics_size);
  return success ? 0 : 1;
}

extern "C" int dynamite_pp(int argc, const char *const *argv,
                            char **output, size_t *output_size,
                            char **diagnostics, size_t *diagnostics_size) {
  return invoke(argc, argv, nullptr, 0, output, output_size, diagnostics, diagnostics_size);
}
extern "C" int dynamite_lower(int argc, const char *const *argv, const char *ir, size_t ir_size,
                               char **output, size_t *output_size,
                               char **diagnostics, size_t *diagnostics_size) {
  return invoke(argc, argv, ir, ir_size, output, output_size, diagnostics, diagnostics_size);
}
