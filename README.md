# cormanlisp-rs

An experimental port of [Corman Lisp](https://github.com/sharplispers/cormanlisp)
to **64-bit x86 Linux**, with a kernel written in Rust.

Corman Lisp was originally a Common Lisp implementation for 32-bit Windows,
with a C++ kernel and much of its standard library written in Lisp. This
project replaces the kernel, compiler backend, and platform-dependent runtime
while reusing and adapting the upstream Lisp sources. It runs natively on
Linux; it is not a Wine wrapper around the Windows implementation.

The result is a working but incomplete Common Lisp system. It boots into a
Lisp-level REPL, compiles Lisp forms to native x86-64 code, and can save and
restore heap images. It is not yet an ANSI-conforming or production-ready
Common Lisp implementation. Correctness and bootstrapping take priority over
performance.

## Build and run

Requirements:

- x86-64 Linux. Other operating systems and architectures are not supported.
- A current stable Rust toolchain (edition 2024), Cargo, and a system C linker.

Run these commands from the repository root:

```sh
cargo build

# Load the Lisp system and enter its REPL.
./target/debug/cormanlisp --boot lisp/boot.lisp --repl
```

The Lisp sources needed for bootstrapping are included in `lisp/sys/`; a
separate upstream checkout is not needed for normal use.

To evaluate a form without entering the REPL:

```sh
./target/debug/cormanlisp --boot lisp/boot.lisp \
  -e '(loop for i from 1 to 5 collect (* i i))'
# Final result: (1 4 9 16 25)
```

Bootstrapping prints progress messages as it loads the system. Save an image
to avoid repeating that work on each invocation:

```sh
./target/debug/cormanlisp --boot lisp/boot.lisp --save cormanlisp.img
./target/debug/cormanlisp --image cormanlisp.img
./target/debug/cormanlisp --image cormanlisp.img -e '(format t "Hello, Lisp!~%")'
```

`--image` must be the first option. Images are implementation-specific heap
snapshots, not portable compiled files. Rebuild them after compiler or runtime
changes; incompatible kernel identifiers are rejected. Open file streams are
not restored as live operating-system handles.

Without `--boot` or `--image`, the executable starts a minimal kernel world,
not the full Lisp system. Use `--repl` to enter the REPL after loading or
evaluating forms. `--disasm` enables compiler disassembly output.

## How it works

The Rust kernel supplies the object representation, memory allocation,
garbage collection, native compiler, Lisp/Rust calling convention, and core
primitives. Evaluation compiles forms to machine code rather than interpreting
them. The compiler uses `iced-x86` for code generation and disassembly; Linux
interfaces are accessed through `libc`.

During bootstrap, the Lisp implementations of the reader, printer, macros,
standard library, and top level take over from the minimal kernel facilities.
Upstream 32-bit assembly routines are replaced with Rust primitives or Lisp
code. The original Lisp-side x86 compiler and assembler are not used.

The runtime uses 64-bit tagged objects and a mostly-copying garbage collector
with conservative stack roots. It currently supports a single Lisp thread.

## Status and remaining work

### Working today

- Native compilation, lexical closures, macros, special bindings, multiple
  values, and non-local control flow, including `catch`, `throw`,
  `unwind-protect`, and `progv`.
- A bootstrapped Lisp reader, printer, REPL, and substantial standard-library
  functionality: packages, structures, arrays, hash tables, sequences,
  strings, formatting, and extended `loop`.
- CLOS and the upstream condition system, including generic functions,
  methods, handlers, and restarts, with limitations noted below.
- Fixnums, ratios with fixnum components, and single- and double-precision
  floating-point numbers.
- Garbage collection, image save/load, file input, and several Lisp stream
  types, including string streams.

### Incomplete or not implemented

- **Numeric coverage:** no bignums or complex numbers. Integer overflow is an
  error rather than promotion to a bignum. Short floats are not a distinct
  supported representation.
- **Files and paths:** complete Linux file I/O and pathname support remain to
  be implemented; much of the upstream code assumes Win32.
- **Compiled files:** no `compile-file`/FASL support. Loading source compiles
  forms in memory; saved images are the current persistence mechanism.
- **Compiler completeness and optimization:** compiler macros are recorded
  but not applied, and inline expansion is not implemented. The Lisp-side
  assembler/compiler would need an x86-64 port to support upstream `defasm`.
- **CLOS lexical environments:** `capture-compiler-environment` is not
  implemented, so method bodies are compiled in the null lexical environment.
  A `defmethod` inside a `let` cannot close over that binding.
- **Diagnostics and conformance:** useful stack traces and more precise
  kernel error conditions are needed. Kernel errors currently become
  `simple-error` conditions. Known reader gaps include non-nesting
  `#| ... |#` comments.
- **Runtime facilities:** no Lisp threads, Linux FFI, weak-pointer semantics,
  or finalization. Weak pointers are currently treated as strong references.
- **Testing:** broader ANSI coverage and fixes for the remaining failures are
  needed before making conformance claims.

The latest result for the upstream HyperSpec example tests is **539 passed, 8
failed, and 8 errors**. This is a limited example suite, not a complete ANSI
conformance test suite.

The Windows IDE, COM integration, Win32/winsock/registry APIs, PE images, and
Windows hardware-assisted write barrier are outside the scope of this port.

## Tests

Run the Rust and Lisp integration tests:

```sh
cargo test
```

To stress garbage collection and verify heap pointers after each collection:

```sh
CORMANLISP_GC_THRESHOLD=4k CORMANLISP_GC_VERIFY=1 cargo test
```

The upstream HyperSpec example tests require a separate Corman Lisp checkout,
a POSIX shell, and the `timeout` command:

```sh
git clone https://github.com/sharplispers/cormanlisp.git ../cormanlisp-upstream
UPSTREAM="$PWD/../cormanlisp-upstream" scripts/ansi-tests.sh

# Optionally select chapters and retain per-chapter output.
UPSTREAM="$PWD/../cormanlisp-upstream" KEEP=/tmp/cormanlisp-tests \
  scripts/ansi-tests.sh 2 3 5
```

The script reports counts for inspection; its exit status is not a conformance
pass/fail gate.

## License and acknowledgments

Original project work is licensed under the **MIT License**. This project
builds on Corman Lisp by Corman Technologies Inc. and its contributors, whose
MIT license is retained. Some bundled Lisp code carries additional notices,
including Closette (Xerox), LOOP (MIT and Symbolics), and CMU Common Lisp code.
Rust dependencies retain their own licenses.

See [LICENSE.txt](LICENSE.txt) for the license texts, attributions, and
redistribution notices. The license for original work does not replace the
licenses or notices attached to upstream code.
