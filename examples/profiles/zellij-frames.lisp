;; Pure pane-frame decoration helper for the pinned Zellij 0.43.1 theme.
;;
;; Interface:
;;   (zellij-frame-spans plist)
;;
;; PLIST contains :RECT (x y columns rows), :TITLE (a printable Unicode string), :SCROLL
;; (position length), :FOCUS (a boolean), and :MODE (:NORMAL, :LOCKED, or
;; another mode). An optional :SGR overrides the frame rendition, allowing a
;; public pane-note contribution to mark the frame without a privileged path.
;; The result is an ordered list of plists with :X, :Y, :TEXT, and :SGR keys.
;; :SGR is the integer SGR sequence used by Ekko's renderer:
;; (0 1 38 5 color 49) for focused frames or (0 1 39 49) when unfocused.
;;
;; This is intentionally a decoration function, independent of Ekko's scene
;; or input state. It covers the standard, rectangular, non-floating pane
;; frame observed by the custom two-pane/no-bar probe. Stacked, floating,
;; pinned, rounded-corner, mouse-hover, multiplayer, exit-status, one-line,
;; frame-disabled variants are outside this helper's
;; contract and must be added explicitly before use.
;;
;; Tiled geometry and frame colours follow the pinned profile. Header text uses
;; a simple cell-width clip and centre operation instead of upstream UI fitting.

(in-package #:cl-user)

(defparameter *zellij-frame-selected-color* 154)
(defparameter *zellij-frame-highlight-color* 166)

(defun zellij-frame-hidden-p (snapshot)
  (let ((state (cdr (assoc "zellij-frames" (getf snapshot :component-state)
                           :test #'equal))))
    (and (listp state) (getf state :hidden))))

(defun zellij-frame-boundary-spans (snapshot)
  "Shared ordinary tiled boundaries, including focused neighboring edges."
  (let* ((viewport (getf snapshot :viewport))
         (insets (getf viewport :insets))
         (left (fourth insets)) (top (first insets))
         (right (- (getf viewport :cols) (second insets)))
         (bottom (- (getf viewport :rows) (third insets)))
         (focus (getf snapshot :focus))
         (mode (getf snapshot :mode))
         (panes (remove-if-not (lambda (pane) (getf pane :visible t))
                              (getf snapshot :panes)))
         (cells (make-hash-table :test #'equal)))
    (labels ((mark (x y selected)
               (let ((key (list x y)))
                 (setf (gethash key cells) (or selected (gethash key cells) :plain))))
             (present (x y) (gethash (list x y) cells)))
      (dolist (pane panes)
        (destructuring-bind (x y width height) (getf pane :outer-rect)
          (let ((selected (eql (getf pane :id) focus))
                (x-end (+ x width -1)) (y-end (+ y height -1)))
            (dolist (column (append (when (> x left) (list (1- x)))
                                   (when (< (1+ x-end) right) (list x-end))))
              (loop for row from (max top (1- y)) to y-end do
                (mark column row selected)))
            (dolist (row (append (when (> y top) (list (1- y)))
                                (when (< (1+ y-end) bottom) (list y-end))))
              (loop for column from (max left (1- x)) to x-end do
                (mark column row selected))))))
      (loop for key being the hash-keys of cells
            for x = (first key) for y = (second key)
            for up = (present x (1- y)) for down = (present x (1+ y))
            for prev = (present (1- x) y) for next = (present (1+ x) y)
            for glyph = (cond ((and up down prev next) #\┼)
                              ((and up down next) #\├) ((and up down prev) #\┤)
                              ((and prev next down) #\┬) ((and prev next up) #\┴)
                              ((or up down) #\│) (t #\─))
            collect (zellij-frame-span
                     x y (string glyph)
                     (if (eq (gethash key cells) t)
                         (list 0 38 5 (if (member mode '(:normal :locked))
                                          *zellij-frame-selected-color*
                                          *zellij-frame-highlight-color*) 49)
                         '(0 39 49)))))))

(defun zellij-title-whitespace-p (character)
  (let ((code (char-code character)))
    (or (<= #x9 code #xd) (= code #x20) (= code #x85) (= code #xa0)
        (= code #x1680) (<= #x2000 code #x200a) (<= #x2028 code #x2029)
        (= code #x202f) (= code #x205f) (= code #x3000))))

(defun zellij-title-trim (string)
  (let ((start (or (position-if-not #'zellij-title-whitespace-p string) 0))
        (end (or (position-if-not #'zellij-title-whitespace-p string :from-end t) -1)))
    (if (> start end) "" (subseq string start (1+ end)))))

(defun zellij-pane-title (pane)
  "Select the pinned Zellij title from public pane metadata.

Explicit non-empty rename wins, then OSC 0/2 (including an explicit empty
title after Unicode White_Space trimming), then the command display string,
then the implicit shell's Pane # creation position."
  (let ((name (getf pane :name))
        (terminal-title (getf pane :terminal-title))
        (argv (getf pane :argv)))
    (cond
      ((and (stringp name) (plusp (length name))) name)
      ((stringp terminal-title) (zellij-title-trim terminal-title))
      ((and (eq (getf pane :launch-kind) :command) argv)
       (format nil "~{~A~^ ~}" argv))
      (t (format nil "Pane #~D" (or (getf pane :creation-position) 1))))))

(defun zellij-frame-char-width (character)
  "Measure a printable title scalar through the public text API."
  (let ((code (char-code character)))
    (when (or (< code #x20) (<= #x7f code #x9f))
      (error "Control character is unsupported in a frame title: ~S" character))
    (ekko/extensions:display-width character)))

(defun zellij-frame-string-width (string)
  (loop for character across string
        sum (zellij-frame-char-width character)))

(defun zellij-frame-take-width-from-start (string max-width)
  (with-output-to-string (out)
    (loop with width = 0
          for character across string
          for character-width = (zellij-frame-char-width character)
          while (<= (+ width character-width) max-width)
          do (write-char character out)
             (incf width character-width))))

(defun zellij-frame-repeat (character count)
  (make-string (max 0 count) :initial-element character))

(defun zellij-frame-title-line (columns title)
  "A full-width header with its title centred by display cells."
  (let* ((text (zellij-frame-take-width-from-start title (max 0 (- columns 2))))
         (padding (- columns (zellij-frame-string-width text)))
         (left (floor padding 2)))
    (concatenate 'string (make-string left :initial-element #\Space) text
                 (make-string (- padding left) :initial-element #\Space))))

(defun zellij-frame-bottom-line (columns)
  (concatenate 'string "└" (zellij-frame-repeat #\─ (max 0 (- columns 2))) "┘"))

(defun zellij-frame-sgr (focus mode)
  (if focus
      (list 0 1 38 5 (if (member mode '(:normal :locked))
                         *zellij-frame-selected-color*
                         *zellij-frame-highlight-color*)
             49)
      '(0 1 39 49)))

(defun zellij-frame-span (x y text sgr &optional (rows 1))
  (list :x x :y y :text text :sgr sgr :rows rows))

(defun zellij-frame-spans (pane)
  "Return centred solid headers and thin borders as ordinary text spans."
  (destructuring-bind (x y columns rows) (getf pane :rect)
    (let* ((title (or (getf pane :title) ""))
           (focus (getf pane :focus))
           (mode (getf pane :mode))
           (sgr (or (getf pane :sgr) (zellij-frame-sgr focus mode)))
           (last-x (+ x columns -1))
           (last-y (+ y rows -1))
           (spans (list (zellij-frame-span
                         x y (zellij-frame-title-line columns title) (append sgr '(7))))))
      (when (> rows 2)
        (setf spans
              (nconc spans
                     (list (zellij-frame-span x (1+ y) "│" sgr (- rows 2))
                           (zellij-frame-span last-x (1+ y) "│" sgr (- rows 2))))))
      (when (> rows 1)
        (setf spans
              (nconc spans
                     (list (zellij-frame-span
                            x last-y (zellij-frame-bottom-line columns) sgr)))))
      spans)))
