;;; Durable per-component state: one file per namespace, atomic replacement.
;;; The directory lives outside the session runtime directory so the values
;;; survive daemon restarts. Nothing here trusts file contents: values are
;;; read with *READ-EVAL* disabled and bounded before they enter a session.
(defpackage #:ekko/store
  (:use #:cl)
  (:export #:store-directory #:store-file #:store-load #:store-write #:store-namespace))
(in-package #:ekko/store)

(defparameter +namespace-limit+ 256)
(defparameter +file-limit+ (* 64 1024))

(defun store-directory ()
  "Durable state location. EKKO_STORE_DIR overrides XDG_STATE_HOME."
  (or (let ((override (uiop:getenv "EKKO_STORE_DIR")))
        (when override (uiop:ensure-directory-pathname override)))
      (let ((state (uiop:getenv "XDG_STATE_HOME")))
        (if (and state (plusp (length state)))
            (merge-pathnames "ekko/store/" (uiop:ensure-directory-pathname state))
            (merge-pathnames ".local/state/ekko/store/" (user-homedir-pathname))))))

(defun store-namespace (owner)
  "Namespace string for a component owner, or NIL without one."
  (when owner
    (let ((name (string-downcase (string owner))))
      (when (> (length name) +namespace-limit+) (error "Store namespace is too long"))
      name)))

(defun store-file (namespace)
  ;; Percent-encode everything outside a safe set so a namespace can never
  ;; traverse out of the store directory or collide by case.
  (when (zerop (length namespace)) (error "Store namespace is empty"))
  (let ((name (with-output-to-string (out)
                (loop for c across namespace
                      do (if (or (alphanumericp c) (find c "-_." :test #'char=))
                             (write-char c out)
                             (format out "%~2,'0X" (char-code c)))))))
    (merge-pathnames (concatenate 'string name ".lisp") (store-directory))))

(defun private-directory (path)
  ;; The store is private to the user; refuse to write through a directory
  ;; owned by somebody else, mirroring the runtime and asset directories.
  (ensure-directories-exist path)
  (let* ((name (string-right-trim "/" (namestring path)))
         (st (sb-posix:lstat name)))
    (unless (and (sb-posix:s-isdir (sb-posix:stat-mode st))
                 (= (sb-posix:stat-uid st) (sb-posix:getuid)))
      (error "Unsafe store directory ~A" name))
    (sb-posix:chmod name #o700)
    path))

(defun read-entry (path)
  "Return (namespace . entries) for one store file, or NIL when unusable."
  (handler-case
      (let ((size (with-open-file (in path :element-type '(unsigned-byte 8))
                    (file-length in))))
        (when (> size +file-limit+) (error "Store file exceeds 64 KiB"))
        (let ((data (with-open-file (in path)
                      (with-standard-io-syntax
                        (let ((*read-eval* nil) (*package* (find-package :ekko/store)))
                          (read in nil nil))))))
          (destructuring-bind (marker version namespace entries) data
            (unless (and (eq marker :ekko-store) (eql version 1)
                         (stringp namespace) (<= (length namespace) +namespace-limit+)
                         (listp entries))
              (error "Invalid store file"))
            (unless (loop for entry in entries
                          always (and (consp entry)
                                      (or (stringp (car entry)) (keywordp (car entry)))))
              (error "Invalid store entry"))
            (cons namespace entries))))
    (error (e) (format *error-output* "store: ~A: ~A~%" path e) nil)))

(defun store-load ()
  "Load every readable namespace in the store directory."
  (let ((directory (store-directory)))
    (when (probe-file directory)
      (loop for path in (directory (merge-pathnames "*.lisp" directory))
            for entry = (read-entry path)
            when entry collect entry))))

(defun store-write (namespace entries)
  "Replace one namespace atomically. Signals on any filesystem failure."
  (let* ((path (store-file namespace))
         (temp (concatenate 'string (namestring path) ".tmp")))
    (private-directory (store-directory))
    (with-open-file (out temp :direction :output :if-exists :supersede :if-does-not-exist :create)
      (with-standard-io-syntax
        (let ((*print-readably* nil) (*print-pretty* nil))
          (write (list :ekko-store 1 (coerce namespace 'simple-string) entries) :stream out)))
      (finish-output out))
    (sb-posix:rename temp (namestring path))
    path))
