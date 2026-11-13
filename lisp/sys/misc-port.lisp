;;;; PORT: portable definitions copied from upstream files that are not
;;;; loaded yet (misc-features.lisp and math2.lisp mostly depend on kernel
;;;; internals, CLOS or complex numbers). Drop each piece when its file is
;;;; brought up.

(in-package "COMMON-LISP")

;;; From misc-features.lisp.
;;;
;;;	Common Lisp ASSERT macro
;;;
(defmacro assert (test-form &rest rest)
	(let ((places (first rest))
		   (msg    (second rest))
		   (args   (cddr rest)))
		(declare (ignore places))
		(if (null msg)
			`(unless ,test-form (error "Assertion failed: ~A" ',test-form))
			`(unless ,test-form (error ,msg ,@args)))))

;; REMOVE-PROPERTY
;; Returns two values: 
;;		the (possibly) modified list
;;		t if the values was found, nil otherwise
;;
(defun remove-property (plist property)
	(cond ((null plist) (values plist nil))
		  ((eq (car plist) property) (values (cddr plist) t))
		  (t (do* ((p (cddr plist) (cddr p))
				   (q plist (cddr q)))
				  ((null p) (values plist nil))
				(when (eq (car p) property)
					(setf (cdr (cdr q)) (cddr p)) 
					(return (values plist t)))))))

;;; Common Lisp REMF function.
(defmacro REMF (place indicator)
	(let ((list-sym (gensym))
		  (flag-sym (gensym)))			
		(if (not (consp place))
			`(multiple-value-bind (,list-sym ,flag-sym)
				(remove-property ,place ,indicator)
				(when ,flag-sym
					(setf ,place ,list-sym)
					,flag-sym))
			(let* ((ret (cl::%once-only-forms place))
				   (let-vars (caar ret))
				   (set-form (cdr ret)))
				(if let-vars
					`(let (,let-vars)
						(multiple-value-bind (,list-sym ,flag-sym)
							(remove-property ,set-form ,indicator)
							(when ,flag-sym
								(setf ,set-form ,list-sym)
								,flag-sym)))
					`(multiple-value-bind (,list-sym ,flag-sym)
						(remove-property ,place ,indicator)
						(when ,flag-sym
							(setf ,place ,list-sym)
							,flag-sym)))))))

;;; Common Lisp REMPROP function.
(defun REMPROP (symbol indicator)
	(remf (symbol-plist symbol) indicator))

;;;
;;; COPY-SYMBOL implementation by Frank Adrian
;;;
(defun copy-symbol (sym &optional copy-props)
	(let ((new-sym (make-symbol (symbol-name sym))))
		(when copy-props
			(when (boundp sym)
				(setf (symbol-value new-sym) (symbol-value sym)))
			(when (fboundp sym)
				(setf (symbol-function new-sym) (symbol-function sym)))				
			(let ((plist (symbol-plist sym)))
				(when plist (setf (symbol-plist new-sym) plist))))
		new-sym))

(defun equalp-arrays (x y)
	(unless (equalp (array-dimensions x)(array-dimensions y))
		(return-from equalp-arrays nil))
	(dotimes (i (array-total-size x))
		(unless (equalp (row-major-aref x i) (row-major-aref y i))
			(return-from equalp-arrays nil)))
	t)

;;; Assume two structures as input, return true iff both contain the same
;;; slot contents (under equalp).	
(defun equalp-structures (x y)
    (let ((num-slots-x (uvector-num-slots x))
          (num-slots-y (uvector-num-slots y)))
        (if (/= num-slots-x num-slots-y)
            nil
            (dotimes (i num-slots-x t)
                (unless (equalp (uref x (+ i 1)) (uref y (+ i 1)))
                    (return-from equalp-structures nil))))))
    
(defun equalp-hash-tables (x y)
  (and (eql (hash-table-test x) (hash-table-test y))
       (eql (hash-table-count x) (hash-table-count y))
       (with-hash-table-iterator (next x)
         (loop
            (multiple-value-bind (more x-key x-value)
                (next)
              (if more
                  (multiple-value-bind (y-value foundp)
                      (gethash x-key y)
                    (unless (and foundp (equalp x-value y-value))
                      (return nil)))
                  (return t)))))))

(defun equalp (x y)
	(cond ((equal x y)	t)
		  ((and (characterp x) (characterp y)) 	(char-equal x y))
		  ((and (numberp x)(numberp y)) 		(= x y))
		  ((and (consp x)(consp y))				(and (equalp (car x)(car y))(equalp (cdr x)(cdr y))))
		  ((and (stringp x)(stringp y))			(string-equal x y))
		  ((and (arrayp x)(arrayp y))			(equalp-arrays x y))
		  ((and (hash-table-p x)(hash-table-p y)) (equalp-hash-tables x y))
		  ((and (structurep x)(structurep y))	(equalp-structures x y))
   		  (t nil)))

;;; From math2.lisp.
;;;;
;;;; Common Lisp LOGBITP function
;;;;
(defun logbitp (index integer)
	(unless (integerp integer)
		(error 'type-error :datum integer :expected-type 'integer))
	(unless (and (fixnump index) (>= index 0))
		(error 'type-error :datum index :expected-type '(integer 0 *)))
	(> (logand integer (expt 2 index)) 0))

;;;;
;;;; Common Lisp LOGCOUNT function
;;;;
(defun logcount (integer)
	(unless (integerp integer)
		(error 'type-error :datum integer :expected-type 'integer))
	;; if negative, use two's complement to flip
	(do ((x (if (< integer 0) (- (+ integer 1)) integer)(ash x -1))
		 (count 0 (+ count (logand x 1))))
		((= x 0) count)))

;;;;
;;;; Common Lisp LDB function
;;;;
(defun ldb (bytespec integer)
	(let ((mask (- (ash 1 (byte-size bytespec)) 1)))
		(logand (ash integer (- (byte-position bytespec))) mask)))

;;;;
;;;; Common Lisp (SETF LDB) function
;;;;
(defmacro |(SETF LDB)| (new-byte bytespec place)
	(if (and (consp place) (some #'consp place))
		(let ((retval (%once-only-forms place))
			  (sym (gensym)))
			`(let ,(car retval)
				(let ((,sym ,new-byte)) 
					(setf ,(cdr retval) (dpb ,sym ,bytespec ,(cdr retval)))
					,sym)))
		(let ((sym (gensym)))
			`(let ((,sym ,new-byte))
				(setf ,place (dpb ,sym ,bytespec ,place))
				,sym))))

(register-setf-function 'ldb '|(SETF LDB)|)


;;;;
;;;; Common Lisp LDB-TEST function
;;;;
(defun ldb-test (bytespec integer)
	(/= (ldb bytespec integer) 0))

;;; Common Lisp GCD function (a kernel function upstream; fixnums only).
(defun gcd (&rest integers)
	(let ((result 0))
		(dolist (n integers (abs result))
			(unless (integerp n)
				(error 'type-error :datum n :expected-type 'integer))
			(do ((a (abs result))
				 (b (abs n)))
				((zerop b) (setq result a))
				(psetq a b b (mod a b))))))

;;; Common Lisp FRESH-LINE (upstream gets it from xp.lisp).
(defun fresh-line (&optional output-stream)
	(if (or (null output-stream) (eq t output-stream))
		(setq output-stream *standard-output*))
	(unless (= (stream-column output-stream) 0)
		(terpri output-stream)
		t))

;;; From misc-features.lisp: functions for accessing function properties.
;;; KERNEL-FUNCTION-P is a kernel function upstream.
(defun ccl::kernel-function-p (func)
	(and (uvectorp func) (eq (uvector-type-bits func) uvector-kfunction-tag)))
;;; functions for accessing function properties
;;;
(defun function-code-buffer (func)
	(check-type func function)
	(if (ccl::kernel-function-p func)
		nil
		(uref func cl::function-code-buffer-offset)))

(defun kernel-function-lambda-list (func)
	(declare (ignore func))
	nil)		;; not implemented yet

(defun function-info-list (func)
	(let ((cb (function-code-buffer func)))
		(if cb
			(uref cb cl::compiled-code-info-offset))))

(defun (setf function-info-list) (val func)
	(let ((cb (function-code-buffer func)))
		(if cb
			(setf (uref cb cl::compiled-code-info-offset) val))))
	
(defun function-lambda-list (func) (getf (function-info-list func) 'cl::lambda-list))
(defun function-lambda (func) (getf (function-info-list func) 'cl::lambda))
(defun function-source-file (func) (getf (function-info-list func) 'ccl:*source-file*))
(defun function-source-line (func) (getf (function-info-list func) 'ccl:*source-line*))
(defun function-documentation (func) (getf (function-info-list func) 'documentation))
(defun (setf function-documentation) (val func)
   (if val (setf (getf (function-info-list func) 'documentation) val) (remf (function-info-list func) 'documentation)) val)
(defun macro-lambda-list (func) (getf (function-info-list func) 'cl::macro-lambda-list))
(defun (setf macro-lambda-list) (val func) 
	(setf (getf (function-info-list func) 'cl::macro-lambda-list)
		val))
