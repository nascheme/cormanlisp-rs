;;;;	-------------------------------
;;;;	Copyright (c) Corman Technologies Inc.
;;;;	See LICENSE.txt for license information.
;;;;	-------------------------------
;;;;
;;;; fast-class-of.lisp -- Fast CLASS-OF reimplementation for Corman Lisp.
;;;; This should be loaded AFTER Closette is loaded.  Benchmarks show
;;;; generic function call speed-up by a factor of 2 and slot access
;;;; speed-up by a factor of 3.  NOTE: class metaobjects are captured in
;;;; variables and therefore must not be replaced after this is loaded.

;;;; Portions Copyright (C) 1999 Vassili Bykov.

;;;; Author:       Vassili Bykov <vassili@objectpeople.com>
;;;; Created:      01/21/1999
;;;; Last updated: 01/21/1999
;;;;
;;;; 			   01/23/1999  Roger Corman  Modified to support changes
;;;;						   in version 1.2, and replaces some numeric
;;;;						   constants with symbolic names.
;;;;			   12/06/1999  Added Ratio class.
;;;;			   02/15/2001  Changed the way a structure's class is looked up.
;;;;

(in-package :common-lisp)

(defvar %*magic-class-table*
  (vector
   (find-class 'function)				; 00000
   (find-class 'function)				; 00001
   nil									; 00010 - a structure, never looked up
   (find-class 'array)					; 00011
   nil									; 00100 - a symbol, never looked up
   (find-class 'stream)					; 00101
   (find-class 'float)					; 00110
   (find-class 'package)				; 00111
   (find-class 'hash-table)				; 01000
   (find-class t)						; 01001 - foreign pointer
   (find-class t)						; 01010 - compiled code
   (find-class 'readtable)				; 01011
   (find-class 'complex)				; 01100
   (find-class 'ratio)					; 01101 - ratio
   (find-class 'integer)				; 01110 - bignum
   (find-class t)						; 01111 - foreign heap pointer
   (find-class t)						; 10000 - weak pointer
   (find-class 'vector)					; 10001 - simple vector
   (find-class 'string)					; 10010
   (find-class 'vector)					; 10011 - simple byte vector
   (find-class 'vector)					; 10100 - simple short vector
   (find-class 'vector)					; 10101 - simple double float vector
   (find-class 'bit-vector)				; 10110
   (find-class 'vector)					; 10111 - simple single float vector
   (find-class 'float)					; 11000 - single float
   :reserved							; 11001
   :reserved							; 11010
   :reserved							; 11011
   :reserved							; 11100
   :reserved							; 11101
   :reserved							; 11110
   :reserved							; 11111
   (find-class 'cons)					; dec 32
   (find-class 'integer)				; dec 33
   (find-class 'character)				; dec 34
   (find-class 'float)))				; dec 35

(defun %class-from-magic-cookie (cookie)
  (declare (optimize (speed 3) (safety 0))
		   (type fixnum cookie))
  (svref %*magic-class-table* cookie))

(defvar %t-class (find-class 't))
(defvar %null-class (find-class 'null))
(defvar %symbol-class (find-class 'symbol))
(defvar %struct-class (find-class 'structure-object))

;; PORT: DEFASM replaced by Lisp.
(defun class-of (object)
  (cond ((consp object) (%class-from-magic-cookie 32))
        ((fixnump object) (%class-from-magic-cookie 33))
        ((characterp object) (%class-from-magic-cookie 34))
        ((not (uvectorp object)) (error "Wrong object in CLASS-OF"))
        (t
         (let ((cookie (uvector-type-bits object)))
           (cond ((eq cookie uvector-clos-instance-tag)
                  (uref object clos-instance-class-offset))
                 ((eq cookie uvector-structure-tag)
                  (let ((template (uref object 1)))
                    (if (symbolp template)
                        %struct-class
                        (uref template (+ 2 struct-template-class-offset)))))
                 ((eq cookie uvector-symbol-tag)
                  (if (eq object nil) %null-class %symbol-class))
                 (t (%class-from-magic-cookie cookie)))))))
	

