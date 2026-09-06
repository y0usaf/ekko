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
;; The title and scroll fitting code below is derived from the MIT-licensed
;; Zellij project, zellij-server/src/ui/pane_boundaries_frame.rs (0.43.1),
;; especially render_title_left_side, render_scroll_indication, and the
;; title-line assembly functions. Copyright and license text are retained in
;; tests/zellij/reference/LICENSE.md.

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

(defun zellij-frame-take-width-from-end (string max-width)
  (with-output-to-string (out)
    (let ((width 0)
          (kept '()))
      (dolist (character (reverse (coerce string 'list)))
        (let ((character-width (zellij-frame-char-width character)))
          (when (> (+ width character-width) max-width)
            (return))
          (push character kept)
          (incf width character-width)))
      (dolist (character kept)
        (write-char character out)))))

(defun zellij-frame-title-left (title max-width)
  "Port render_title_left_side, returning (text width) or NIL."
  (let* ((middle-truncated-sign "[..]")
         (middle-truncated-sign-long "[...]")
         (full-text (format nil " ~A " title)))
    (cond
      ((or (<= max-width 6) (zerop (length title))) nil)
      ((<= (zellij-frame-string-width full-text) max-width)
       (list full-text (zellij-frame-string-width full-text)))
      (t
       (let* ((half (floor (- max-width
                              (zellij-frame-string-width middle-truncated-sign))
                           2))
              (first-part (zellij-frame-take-width-from-start full-text half))
              (second-part (zellij-frame-take-width-from-end full-text half))
              (short-width (+ (zellij-frame-string-width first-part)
                              (zellij-frame-string-width middle-truncated-sign)
                              (zellij-frame-string-width second-part))))
         (if (< short-width max-width)
             (list (format nil "~A~A~A" first-part middle-truncated-sign-long
                           second-part)
                   (1+ short-width))
             (list (format nil "~A~A~A" first-part middle-truncated-sign
                           second-part)
                   short-width)))))))

(defun zellij-frame-scroll-right (scroll max-width)
  "Port render_scroll_indication for selectable tiled panes."
  (destructuring-bind (position length) scroll
    (when (or (> position 0) (> length 0))
      (let* ((prefix " SCROLL: ")
             (full (format nil " ~D/~D " position length))
             (short (format nil " ~D " position))
             (prefix-length (length prefix))
             (full-length (length full))
             (short-length (length short)))
        (cond
          ((<= (+ prefix-length full-length) max-width)
           (list (concatenate 'string prefix full) (+ prefix-length full-length)))
          ((<= full-length max-width) (list full full-length))
          ((<= short-length max-width) (list short short-length))
          (t nil))))))

(defun zellij-frame-repeat (character count)
  (make-string (max 0 count) :initial-element character))

(defun zellij-frame-title-line (columns title scroll)
  (let* ((total (max 0 (- columns 2)))
         (left (zellij-frame-title-left title total))
         (left-text (and left (first left)))
         (left-length (if left (second left) 0))
         (right (and left
                     (zellij-frame-scroll-right
                      scroll (max 0 (- total left-length 1)))))
         (right-text (and right (first right)))
         (right-length (if right (second right) 0)))
    (cond
      ((and left right)
       (concatenate 'string "┌" left-text
                    (zellij-frame-repeat #\─ (- total left-length right-length))
                    right-text "┐"))
      (left
       (concatenate 'string "┌" left-text
                    (zellij-frame-repeat #\─ (- total left-length)) "┐"))
      (t
       (concatenate 'string "┌" (zellij-frame-repeat #\─ total) "┐")))))

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
  "Return the pinned Zellij rectangular frame as ordered text spans."
  (destructuring-bind (x y columns rows) (getf pane :rect)
    (let* ((title (or (getf pane :title) ""))
           (scroll (or (getf pane :scroll) '(0 0)))
           (focus (getf pane :focus))
           (mode (getf pane :mode))
           (sgr (or (getf pane :sgr) (zellij-frame-sgr focus mode)))
           (last-x (+ x columns -1))
           (last-y (+ y rows -1))
           (spans (list (zellij-frame-span
                         x y (zellij-frame-title-line columns title scroll) sgr))))
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
