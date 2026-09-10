;;; Focused dock-capacity tests: whole-tile placement and the always-visible
;;; overflow chip. Loads examples/profiles/desktop-style.lisp against a
;;; minimal ekko.extensions stub, then exercises dock() and the
;;; desktop-window-list handler directly with 16 panes.

(defpackage #:ekko/extensions
  (:use #:cl)
  (:export #:display-width #:value #:action #:register-command #:set-option
           #:register-component #:bind-key #:register-keymap))
(in-package #:ekko/extensions)

(defun width-of (c)
  (let ((code (char-code c)))
    (cond ((< code 128) 1)
          ((and (>= code #x300) (< code #x370)) 0)
          (t 1))))

(defun display-width (object)
  (cond ((stringp object) (loop for c across object sum (width-of c)))
        ((characterp object) (width-of object))
        (t (error "display-width expects text"))))

(defun value (snapshot key) (getf snapshot key))

(defun action (name &rest properties) (cons name properties))

(defvar *commands* nil)
(defun register-command (&key component name handler)
  (declare (ignore component))
  (setf (getf *commands* (intern (string-upcase name) :keyword)) handler))
(defun register-component (&key id &allow-other-keys) (declare (ignore id)))
(defun set-option (&key component name value) (declare (ignore component name value)))
(defun bind-key (&key &allow-other-keys) nil)
(defun register-keymap (&key &allow-other-keys) nil)

(defpackage #:dock-capacity-test
  (:use #:cl))
(in-package #:dock-capacity-test)

(defvar *failures* nil)
(defparameter *checks* 0)
(defun check (ok message)
  (incf *checks*)
  (unless ok (push message *failures*) (format t "FAIL: ~A~%" message)))

(defun load-profile ()
  ;; desktop-style.lisp defines its own packages; loading it after our
  ;; ekko/extensions stub reuses that package and registers handlers.
  (load (merge-pathnames "../examples/profiles/desktop-style.lisp"
                         (or *load-truename* *load-pathname*
                             (truename "dock_capacity.lisp"))))
  (funcall (find-symbol "INSTALL-CONTROLS" :ekko/desktop) :test-owner))

(defun pane (&key id minimized title activity)
  (list :id id :minimized minimized :title (or title (format nil "Pane ~D" id))
        :activity activity :name nil :terminal-title nil
        :launch-kind :command :argv (list "p") :activation-order id))

(defun snapshot (width height &optional (mode :normal))
  (let ((panes (loop for id from 1 to 16
                     collect (pane :id id :minimized (eql id 16)
                                   :activity (= 0 (mod id 4))))))
    (list :viewport (list :cols width :rows height :insets '(0 0 1 0))
          :panes panes :focus 1 :mode mode :session "sess"
          :time (get-universal-time))))

(defun dock-spans (width height &optional (mode :normal))
  (let ((actions (funcall (symbol-function (find-symbol "DOCK" :ekko/desktop))
                          (snapshot width height mode) nil)))
    (getf (rest (first actions)) :spans)))

(defun tile-ids (spans)
  (loop for s in spans when (and (getf s :action)
                                 (member (first (getf s :action)) '(:focus :minimize :restore)))
          collect (third (getf s :action))))

(defun overflow-span (spans)
  (find "desktop-window-list" spans
        :test #'equal :key (lambda (s) (getf s :command))))

(defun in-viewport-p (span width)
  (let ((x (getf span :x)) (text (getf span :text)))
    (and (integerp x) (>= x 0)
         (<= (+ x (ekko/extensions:display-width text)) width))))

(defun expected-chip (hidden)
  (format nil " +~D " hidden))

(defun run ()
  ;; Wide: all 16 tiles fit, no overflow chip.
  (let* ((spans (dock-spans 200 40))
         (ids (tile-ids spans)))
    (check (= 16 (length ids)) (format nil "wide shows all tiles: ~D" (length ids)))
    (check (equal ids (loop for i from 1 to 16 collect i)) "wide tile order 1..16")
    (check (null (overflow-span spans)) "wide has no overflow chip")
    (let ((entry (find 1 spans :key (lambda (s) (third (getf s :action))))))
      (check (string= "desktop-cycle" (getf entry :wheel-command))
             "tile carries the wheel cycle command")
      (check (string= "desktop-close" (getf entry :middle-command))
             "tile carries the middle-click close command")
      (check (string= "desktop-window-menu" (getf entry :context-command))
             "tile right-click opens the window menu")))
  ;; 120 wide: still fits (tiles shrink), no overflow chip.
  (let* ((spans (dock-spans 120 40))
         (ids (tile-ids spans)))
    (check (= 16 (length ids)) (format nil "120 shows all tiles: ~D" (length ids)))
    (check (null (overflow-span spans)) "120 has no overflow chip"))
  ;; Narrow widths: whole tiles only, plus an overflow chip whose whole
  ;; span lies inside the viewport and whose count matches the hidden panes.
  (dolist (width '(30 40))
    (let* ((spans (dock-spans width 40))
           (ids (tile-ids spans))
           (chip (overflow-span spans)))
      (check (< (length ids) 16) (format nil "~D clips tiles: ~D" width (length ids)))
      (check (not (null chip)) (format nil "~D shows overflow chip" width))
      (when chip
        (check (equal "desktop-window-list" (getf chip :command))
               (format nil "~D chip left-click command" width))
        (check (null (getf chip :action)) (format nil "~D chip has no :action" width))
        (check (in-viewport-p chip width)
               (format nil "~D chip inside viewport" width))
        (check (equal (expected-chip (- 16 (length ids))) (getf chip :text))
               (format nil "~D chip count matches hidden panes" width))
        (check (every (lambda (s) (<= (+ (getf s :x)
                                         (ekko/extensions:display-width (getf s :text)))
                                      (getf chip :x)))
                      (remove-if-not (lambda (s) (getf s :action)) spans))
               (format nil "~D tiles stop before the chip" width)))))
  ;; Handler: the chip lists every window, including the hidden ones.
  (let* ((handler (getf ekko/extensions::*commands* :desktop-window-list))
         (actions (funcall handler (snapshot 30 40) (list :x 0 :y 40))))
    (check (eq :show-menu (first (first actions))) "chip opens the window list menu")
    (let* ((menu (first actions))
           (spans (getf (rest menu) :spans))
           (titles (remove-if-not (lambda (s) (eql (char (getf s :text) 0) #\│)) spans)))
      (check (= 16 (length titles)) (format nil "menu lists all 16 panes: ~D" (length titles)))
      (check (= (getf (rest menu) :x) 2) "menu is centred for a 30-column viewport")
      (check (every (lambda (s) (in-viewport-p s 30)) spans)
             "menu stays inside the cramped viewport")))
  ;; Pure: rendering the dock leaves the snapshot untouched.
  (let ((snap (snapshot 30 40))
        (copy (copy-tree (snapshot 30 40))))
    (dock-spans 30 40)
    (check (equal snap copy) "snapshot values are unchanged")))

(defun main ()
  (load-profile)
  (run)
  (if *failures*
      (progn (format t "Dock capacity tests FAILED (~D checks): ~A~%"
                     *checks* (reverse *failures*))
             (sb-ext:exit :code 1))
      (progn (format t "Dock capacity tests passed (~D checks)~%" *checks*)
             (sb-ext:exit :code 0))))
