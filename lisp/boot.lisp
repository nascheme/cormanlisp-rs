;;;; Boot script for the Rust kernel: replacement for Sys/compile-sys.lisp
;;;; and Sys/load-sys.lisp. Paths are relative to this file (the kernel
;;;; runs --boot files with their directory as the current directory).
;;;; Files in lisp/sys are copies of upstream Sys/*.lisp; local edits are
;;;; marked with ";; PORT:" comments.

(setq *compiler-save-lambdas* nil)
(setq *compiler-save-table-references* nil)
(setq *append-refs-to-code* t)
(load "sys/bootstrap.lisp")
(load "sys/expand.lisp")
(load "sys/uvector.lisp")
(load "sys/read.lisp")
(load "sys/misc.lisp")
(load "sys/readtable.lisp")
(load "sys/massage.lisp")
(load "sys/write.lisp")

(load "sys/destructure.lisp")
(load "sys/util.lisp")
(load "sys/sequence.lisp")
(load "sys/format.lisp")

(defun load-file (filename)
    (if (eq (cormanlisp-client-type) 2)
        (editor-set-message (format nil "Compiling ~a" filename))
        (progn (format t "Compiling ~a~%" filename)(force-output)))
    (load filename))

(setf *compiler-warn-on-unused-variable* t)
(load-file "sys/types.lisp")
(load-file "sys/io.lisp")
(load-file "sys/clmacros.lisp")
(load-file "sys/backquote.lisp")
(load-file "sys/package.lisp")
(load-file "sys/pl-imports.lisp")
(load-file "sys/setf.lisp")
(load-file "sys/structures.lisp")
(load-file "sys/array.lisp")
(load-file "sys/arrays.lisp")
(load-file "sys/hash-table.lisp")
(load-file "sys/toplevel.lisp")

;; load-sys2.lisp
(load-file "sys/declarations.lisp")
(load-file "sys/kernel-port.lisp")
(load-file "sys/trees.lisp")
(load-file "sys/lists.lisp")
(load-file "sys/characters.lisp")
(load-file "sys/strings.lisp")
(load-file "sys/math.lisp")
(load-file "sys/random.lisp")
(load-file "sys/symbols.lisp")
(load-file "sys/control-structures.lisp")
(load-file "sys/sequences.lisp")
(load-file "sys/subtypep.lisp")
(load-file "sys/coerce.lisp")
(load-file "sys/input-output.lisp")
(load-file "sys/errors.lisp")
(load-file "sys/defpackage.lisp")
(load-file "sys/misc-port.lisp")	; PORT
(load-file "sys/clos.lisp")
(load-file "sys/fast-class-of.lisp")
(load-file "sys/conditions.lisp")
(load-file "sys/streams.lisp")
(load-file "sys/loop.lisp")

;; As compile-sys.lisp: LOAD rebinds *package*, so set it at this level.
(in-package :common-lisp-user)
