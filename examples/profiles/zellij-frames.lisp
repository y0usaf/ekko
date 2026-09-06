;; Pure pane-frame decoration helper for the pinned Zellij 0.43.1 theme.
;;
;; Interface:
;;   (zellij-frame-spans plist)
;;
;; PLIST contains :RECT (x y columns rows), :TITLE (an ASCII string), :SCROLL
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
;; frame-disabled, and non-ASCII-title variants are outside this helper's
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

(defun zellij-frame-char-width (character)
  "Return the pinned title width for the supported ASCII title contract.

Zellij delegates this to unicode-width. The observed and tested profile
titles are ASCII, for which the width is exact; rejecting other characters
keeps truncation deterministic instead of silently approximating their width."
  (let ((code (char-code character)))
    (cond
      ((or (< code #x20) (= code #x7f))
       (error "Control character is unsupported in a frame title: ~S" character))
      ((<= code #x7e) 1)
      (t (error "Non-ASCII title character is unsupported: ~S" character)))))

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
