cmake_minimum_required(VERSION 3.20)

# Find LLVM installation
execute_process(
    COMMAND brew --prefix llvm
    OUTPUT_VARIABLE LLVM_PREFIX
    OUTPUT_STRIP_TRAILING_WHITESPACE
)

set(CMAKE_C_COMPILER "clang")
set(CMAKE_CXX_COMPILER "clang++")
set(CMAKE_C_FLAGS_INIT "-fexperimental-library -Wno-parentheses")
set(CMAKE_CXX_FLAGS_INIT "-nostdlib++ ${CMAKE_C_FLAGS_INIT}")
set(CMAKE_OBJCXX_FLAGS_INIT "-nostdlib++ ${CMAKE_CXX_FLAGS_INIT}")
set(CMAKE_EXE_LINKER_FLAGS_INIT "${LLVM_PREFIX}/lib/c++/libc++.a ${LLVM_PREFIX}/lib/c++/libc++abi.a ${LLVM_PREFIX}/lib/c++/libc++experimental.a")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "${LLVM_PREFIX}/lib/c++/libc++.a ${LLVM_PREFIX}/lib/c++/libc++abi.a ${LLVM_PREFIX}/lib/c++/libc++experimental.a")