;; Shared desktop styling, compiled into defaults and loadable by bare profiles.
;; All interaction uses ordinary, owner-scoped decoration actions.
(defpackage #:ekko/desktop
  (:use #:cl)
  (:export #:windows #:dock #:hook #:install-controls #:*hints*))
(in-package #:ekko/desktop)

(defparameter *hints*
  '((:normal) (:locked ("Ctrl-g" "Unlock"))
    (:pane ("r/d/n" "New pane") ("Tab" "Cycle/restore") ("h/j/k/l" "Focus")
           ("m" "Minimize") ("f" "Maximize") ("c" "Rename") ("x" "Close") ("Esc" "Normal"))
    (:move ("h/j/k/l" "Move pane") ("n/Tab" "Next") ("p" "Previous") ("Esc" "Normal"))
    (:rename ("Type" "Rename pane") ("Enter" "Save") ("Esc" "Cancel"))
    (:session ("d" "Detach") ("Ctrl-q" "Quit") ("Esc" "Normal"))))

(defun safe-text (text)
  (map 'string (lambda (c) (if (or (< (char-code c) 32) (<= 127 (char-code c) 159)) #\Space c))
       (or text "")))

(defun fit-text (text width)
  (with-output-to-string (out)
    (loop with used = 0 for c across text
          for size = (ekko/extensions:display-width c)
          while (<= (+ used size) width)
          do (write-char c out) (incf used size))))

(defun pane-title (pane)
  (let ((name (getf pane :name)))
    (cond ((and name (plusp (length name))) name)
          ((getf pane :terminal-title) (string-trim '(#\Space #\Tab #\Newline #\Return)
                                                   (getf pane :terminal-title)))
          ((eq (getf pane :launch-kind) :command) (format nil "~{~A~^ ~}" (getf pane :argv)))
          (t (format nil "Pane #~D" (getf pane :id))))))

(defun frame-spans (pane)
  (destructuring-bind (x y width height) (getf pane :rect)
    (let* ((title (fit-text (getf pane :title) (max 0 (- width 2))))
           (padding (- width (ekko/extensions:display-width title)))
           (left (floor padding 2)) (sgr (getf pane :sgr))
           (header (concatenate 'string (make-string left :initial-element #\Space) title
                                (make-string (- padding left) :initial-element #\Space))))
      (append (list (list :x x :y y :text header :sgr sgr))
        (when (> height 2)
          (loop for edge in (remove-duplicates (list x (+ x width -1)))
                collect (list :x edge :y (1+ y) :text "│" :sgr sgr :rows (- height 2))))
        (when (> height 1)
          (list (list :x x :y (+ y height -1)
                      :text (if (= width 1) "│"
                                (concatenate 'string "└" (make-string (- width 2) :initial-element #\─) "┘"))
                      :sgr sgr)))))))

(defun desktop-color (id)
  (nth (mod (1- id) 6) '(154 117 213 221 114 209)))

(defun menu-spans (items color)
  (let* ((width 26) (base '(0 38 5 252 48 5 236))
         (edge '(0 38 5 242 48 5 236))
         (hover (list 0 38 5 16 48 5 color)))
    (append
      (list (list :x 0 :y 0 :text (concatenate 'string "┌" (make-string (- width 2) :initial-element #\─) "┐") :sgr edge))
      (loop for (label action command arguments) in items for y from 1
            for text = (fit-text label (- width 4))
            collect (append
              (list :x 0 :y y :text (concatenate 'string "│ " text
                       (make-string (- width 3 (ekko/extensions:display-width text)) :initial-element #\Space) "│")
                    :sgr base :hover-sgr hover)
              (when action (list :action action))
              (when command (list :command command :arguments arguments))))
      (list (list :x 0 :y (1+ (length items))
                  :text (concatenate 'string "└" (make-string (- width 2) :initial-element #\─) "┘") :sgr edge)))))

(defun window-menu (pane focus zoom)
  (let ((id (getf pane :id)) (minimized (getf pane :minimized)))
    (menu-spans
      (append
        (list (list (if minimized "Restore window" "Focus window") (list (if minimized :restore :focus) :pane id)))
        (unless minimized
          (list (list "Minimize" (list :minimize :pane id))
                (list (if (and zoom (eql id focus)) "Restore size" "Maximize") (list :zoom :pane id))))
        (list (list (if (getf pane :floating) "Tile window" "Float window")
                    (list (if (getf pane :floating) :tile :float) :pane id))
              (list "Rename…" nil "desktop-rename" (list (write-to-string id)))
              (list "Split right" (list :split :pane id :axis :columns))
              (list "Split down" (list :split :pane id :axis :rows))
              (list "Close window" (list :close :pane id))))
      (desktop-color id))))

(defun windows (snapshot event)
  (declare (ignore event))
  (let ((focus (ekko/extensions:value snapshot :focus))
        (mode (ekko/extensions:value snapshot :mode)))
    (list (ekko/extensions:action :decorate :spans
      (loop for pane in (ekko/extensions:value snapshot :panes)
            when (getf pane :visible)
            append
        (destructuring-bind (x y width height) (getf pane :outer-rect)
          (let* ((id (getf pane :id)) (controls (>= width 12))
                 (title (fit-text
                         (safe-text (pane-title pane))
                         (max 0 (- width (if controls 20 2)))))
                 (spans (frame-spans
                         (list :rect (list x y width height) :title title
                               :focus (eql id focus) :mode mode
                               :sgr (list 0 (if (eql id focus) 1 22) 38 5 (desktop-color id) 49)))))
            (setf spans
              (loop for span in spans collect
                (append span (list :drag
                    (cond ((= (getf span :y) y) :move)
                          ((not (getf pane :floating)) nil)
                          ((= (getf span :y) (+ y height -1)) :bottom)
                          ((= (getf span :x) x) :left) (t :right))))))
            (setf (getf (first spans) :action) (list :focus :pane id)
                  (getf (first spans) :sgr)
                  (list 0 (if (eql id focus) 1 22) 38 5 (desktop-color id) 48 5 235))
            (let ((arguments (list (write-to-string id))))
              (mapcar (lambda (span) (append span (list :pane id :context-command "desktop-window-menu" :arguments arguments)))
                (append spans
              (when (getf pane :floating)
                (list (list :x x :y (+ y height -1) :text "└" :sgr (getf (first spans) :sgr) :drag :bottom-left)
                      (list :x (+ x width -1) :y (+ y height -1) :text "┘" :sgr (getf (first spans) :sgr) :drag :bottom-right)))
              (loop for edge in (remove-duplicates (list x (+ x width -1)))
                    collect (list :x edge :y y :text "│"
                                  :sgr (list 0 38 5 (desktop-color id) 48 5 235)
                                  :action (list :focus :pane id)))
              (when controls
                (loop for (glyph color action) in '((" _ " 221 :minimize)
                                                    (" □ " 114 :zoom)
                                                    (" × " 203 :close))
                      for offset from (- width 10) by 3
                      collect (list :x (+ x offset) :y y :text glyph
                                    :sgr (list 0 1 38 5 color 48 5 235)
                                    :action (list action :pane id))))))))))))))

(defun dock (snapshot event &optional (hints *hints*))
  (declare (ignore event))
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (width (getf viewport :cols)) (y (1- (getf viewport :rows)))
         (panes (ekko/extensions:value snapshot :panes))
         (focus (ekko/extensions:value snapshot :focus))
         (mode (or (ekko/extensions:value snapshot :mode) :normal))
         (session (ekko/extensions:value snapshot :session))
         (base '(0 38 5 250 48 5 235))
         (segments (list (list " EKKO " '(0 1 38 5 154 48 5 235))
                         (list (format nil "~A │ " (fit-text
                                                     (safe-text session) 16)) base)))
         (used (loop for segment in segments sum (ekko/extensions:display-width (first segment))))
         (tile-width (max 3 (min 24 (floor (max 0 (- width used 2 (max 0 (1- (length panes))))) (max 1 (length panes)))))))
    (when (plusp (third (getf viewport :insets)))
      (setf segments
        (append segments
          (loop for pane in panes
                for id = (getf pane :id)
                for minimized = (getf pane :minimized)
                for index from 0
                append (append
                  (when (plusp index)
                    (list (list " " '(0 38 5 245 48 5 235))))
                  (list (list
                  (format nil " ~A~A " (cond (minimized "_") ((eql id focus) "▸") (t "·"))
                          (fit-text
                           (safe-text (pane-title pane)) (max 0 (- tile-width 3))))
                  (if (not minimized)
                      (list 0 1 38 5 16 48 5 (desktop-color id))
                      (list 0 38 5 (desktop-color id) 48 5 239))
                  (list (cond (minimized :restore) ((eql id focus) :minimize) (t :focus))
                        :pane id)
                  (list (write-to-string id))))))
          (if (eq mode :normal)
              (list (list "  Ctrl-p panes · Ctrl-o session"
                          '(0 38 5 245 48 5 235)))
              (cons (list (format nil "  ~A " (string-upcase (symbol-name mode)))
                          '(0 1 38 5 221 48 5 235))
                    (loop for (key label) in (cdr (assoc mode hints))
                          collect (list (format nil " ~A ~A " key label) base))))))
      (let ((x 0) (spans (list (list :x 0 :y y :text (make-string width :initial-element #\Space) :sgr base :context-command "desktop-taskbar-menu"))))
        (dolist (segment segments)
          (let ((text (fit-text (first segment) (max 0 (- width x)))))
            (when (plusp (length text))
              (push (list :x x :y y :text text :sgr (second segment) :action (third segment)
                          :context-command (if (third segment) "desktop-window-menu" "desktop-taskbar-menu")
                          :arguments (fourth segment)) spans)
              (incf x (ekko/extensions:display-width text)))))
        (list (ekko/extensions:action :decorate :spans (nreverse spans)))))))

(defun hook (snapshot event)
  (list (ekko/extensions:action :decorate :spans
          (append (getf (rest (first (windows snapshot event))) :spans)
                  (getf (rest (first (dock snapshot event))) :spans)))))

(defun install-controls (owner)
  (ekko/extensions:set-option :component owner :name :window-animation-ms :value 160)
  (ekko/extensions:register-command :component owner :name "desktop-window-menu"
    :handler (lambda (snapshot event)
               (let* ((id (parse-integer (first (getf event :arguments))))
                      (pane (find id (ekko/extensions:value snapshot :panes) :key (lambda (p) (getf p :id)))))
                 (unless pane (error "Window no longer exists"))
                 (list (ekko/extensions:action :show-menu :x (getf event :x) :y (getf event :y)
                         :spans (window-menu pane (ekko/extensions:value snapshot :focus)
                                             (ekko/extensions:value snapshot :zoom)))))))
  (ekko/extensions:register-command :component owner :name "desktop-taskbar-menu"
    :handler (lambda (snapshot event)
               (let ((focus (ekko/extensions:value snapshot :focus)))
                 (list (ekko/extensions:action :show-menu :x (getf event :x) :y (getf event :y)
                         :spans (menu-spans
                                  (list (list "New window right" (list :split :pane focus :axis :columns))
                                        (list "New window down" (list :split :pane focus :axis :rows))
                                        (list "Detach session" nil "session-detach" nil)) 154))))))
  (ekko/extensions:register-command :component owner :name "desktop-rename"
    :handler (lambda (snapshot event)
               (let* ((id (parse-integer (first (getf event :arguments))))
                      (copy (copy-list snapshot)))
                 (unless (find id (ekko/extensions:value snapshot :panes) :key (lambda (p) (getf p :id)))
                   (error "Window no longer exists"))
                 (setf (getf copy :focus) id)
                 (append (list (ekko/extensions:action :focus :pane id))
                         (cl-user::zellij-pane-rename-enter-actions copy (string-downcase (string owner)))))))
  (ekko/extensions:register-command :component owner :name "desktop-next"
    :handler (lambda (snapshot event) (declare (ignore snapshot event))
               (list (ekko/extensions:action :focus-next))))
  (ekko/extensions:bind-key :component owner :map :pane :key "Tab" :command "desktop-next")
  (ekko/extensions:register-command :component owner :name "desktop-minimize"
    :handler (lambda (snapshot event) (declare (ignore snapshot event))
               (list (ekko/extensions:action :minimize)
                     (ekko/extensions:action :set-keymap :name :normal))))
  (ekko/extensions:bind-key :component owner :map :pane :key "m" :command "desktop-minimize"))
