FROM rust:1.86.0-slim

RUN apt update
RUN apt install -y lsb-release wget software-properties-common gnupg make
RUN wget https://apt.llvm.org/llvm.sh
RUN chmod +x llvm.sh
RUN ./llvm.sh 15
RUN ln -s /usr/bin/clang-15 /usr/bin/clang
