;;;; PORT: replacement for the parts of Sys/kernel-asm.lisp and
;;;; Sys/kernel-funcs.lisp that load-sys2 files depend on. Upstream
;;;; defines these with DEFASM; here they are kernel primitives (in
;;;; COMMON-LISP) imported into the X86 package, or plain Lisp.

(in-package "COMMON-LISP")

;; Upstream creates this package in assembler.lisp.
(unless (find-package "X86")
    (make-package "ASSEMBLER-X86" :nicknames '("X86")))

(import '(%svref %svchar %svbit %svbyte %svdouble %svsingle %svshort)
    (find-package "X86"))

(dolist (s '(%svref %svchar %svbit %svbyte %svdouble %svsingle %svshort))
    (register-setf-function s (setf-function-symbol (list 'setf s))))
(register-setf-function 'svref '|(SETF SVREF)|)

;;; Common Lisp COPY-LIST function (keeps a dotted tail).
(defun copy-list (list)
    (if (atom list)
        list
        (let* ((head (cons (car list) nil))
               (tail head))
            (do ((x (cdr list) (cdr x)))
                ((atom x) (rplacd tail x) head)
                (rplacd tail (cons (car x) nil))
                (setq tail (cdr tail))))))

;;; From Sys/compiler.lisp (which is otherwise the x86 code generator).
;;; Compiler macros are recorded but the kernel compiler does not apply
;;; them yet.
(defconstant most-positive-fixnum 1152921504606846975)	;; PORT: 61-bit fixnums
(defconstant most-negative-fixnum (- -1 most-positive-fixnum))

(defvar x86::*compiler-macro-table* (make-hash-table))

(defun compiler-macro-function (name &optional environment)
	(declare (ignore environment))
	(gethash name x86::*compiler-macro-table*))

(defun (setf compiler-macro-function) (new-function name &optional environment)
	(declare (ignore environment))
	(setf (gethash name x86::*compiler-macro-table*) new-function))

(defmacro define-compiler-macro (name lambda-list &rest forms)
	(let ((doc nil)
		  (lambda-form nil)
		  (declarations nil)
		  (form-sym (gensym))
		  (env-sym (gensym)))

		;; look for declarations and doc string
		(do* ((f forms (cdr f)))
			((null f) (setq forms f))
			(if (and (stringp (car f)) (null doc) (cdr f))
				(setq doc (car f))
				(if (and (consp (car f)) (eq (caar f) 'declare))
					(push (car f) declarations)
					(progn (setq forms f) (return)))))

		(setq lambda-form 
			`(lambda (,form-sym &optional ,env-sym) 
				(declare (ignore ,env-sym ,@(unless lambda-list '(form))))
				(cl::macro-bind ,lambda-list 
					(if (eq (car ,form-sym) 'funcall) (cdr ,form-sym) ,form-sym)
					,@(nreverse declarations) 
					(block ,name ,@forms))))
		`(progn (setf (compiler-macro-function ',name) (function ,lambda-form))
                                     (setf (ccl::macro-lambda-list (compiler-macro-function ',name)) ',lambda-list)
                                     ,@(when doc `((setf (ccl::function-documentation (compiler-macro-function ',name)) ,doc)))
			 (setf (documentation ',name 'compiler-macro) ,doc) ',name)))
