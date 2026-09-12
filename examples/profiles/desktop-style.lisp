;; Shared desktop styling, compiled into defaults and loadable by bare profiles.
;; All interaction uses ordinary, owner-scoped decoration actions.
(defpackage #:ekko/desktop
  (:use #:cl)
  (:export #:windows #:dock #:hook #:install-controls #:*hints*))
(in-package #:ekko/desktop)

(defparameter *hints*
  '((:normal) (:locked ("Ctrl-g" "Unlock"))
    (:pane ("r/d/n" "New pane") ("Tab" "Cycle/restore") ("h/j/k/l" "Focus")
           ("m" "Minimize") ("f" "Maximize") ("t" "Float/tile") ("c" "Rename") ("x" "Close") ("Esc" "Normal"))
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
        (list (list (if (getf pane :floating) "Tile window   Ctrl-p t" "Float window  Ctrl-p t")
                    (list (if (getf pane :floating) :tile :float) :pane id))
              (list "Rename…" nil "desktop-rename" (list (write-to-string id)))
              (list "Split right" (list :split :pane id :axis :columns))
              (list "Split down" (list :split :pane id :axis :rows))
              (list "Close window" (list :close :pane id))))
      (desktop-color id))))

(defun clock-text (time)
  (multiple-value-bind (second minute hour) (decode-universal-time time)
    (declare (ignore second))
    (format nil "~2,'0D:~2,'0D" hour minute)))

(defun window-list-menu (snapshot)
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (panes (sort (copy-list (ekko/extensions:value snapshot :panes))
                      #'> :key (lambda (pane) (getf pane :activation-order))))
         (items (loop for pane in panes collect
                  (list (format nil "~A ~A" (cond ((getf pane :minimized) "_")
                                                  ((getf pane :activity) "●") (t "·"))
                                (pane-title pane))
                        (list :focus :pane (getf pane :id))))))
    (list (ekko/extensions:action :show-menu
            :x (max 0 (floor (- (getf viewport :cols) 26) 2))
            :y (max 0 (floor (- (getf viewport :rows) (+ 2 (length items))) 2))
            :spans (menu-spans items 154)))))

(defun backdrop (snapshot)
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (cols (getf viewport :cols)) (rows (getf viewport :rows))
         (session (fit-text (safe-text (ekko/extensions:value snapshot :session)) 22))
         (items (list "EKKO desktop"
                      (format nil "session ~A" session)
                      (clock-text (ekko/extensions:value snapshot :time))
                      ""
                      "Ctrl-p n    new window"
                      "Ctrl-p Tab  restore window"))
         (width 32) (height (+ 2 (length items)))
         (x (max 0 (floor (- cols width) 2)))
         (y (max 0 (floor (- (max 1 (1- rows)) height) 2)))
         (edge '(0 38 5 250 48 5 236)) (body '(0 38 5 245 48 5 236)))
    (append
      (list (list :x x :y y :text (concatenate 'string "┌" (make-string (- width 2) :initial-element #\─) "┐") :sgr edge))
      (loop for item in items for row from (1+ y)
            for text = (fit-text item (- width 4))
            collect (list :x x :y row
                          :text (concatenate 'string "│ " text
                                             (make-string (max 0 (- width 3 (ekko/extensions:display-width text)))
                                                          :initial-element #\Space)
                                             "│")
                          :sgr body))
      (list (list :x x :y (+ y 1 (length items))
                  :text (concatenate 'string "└" (make-string (- width 2) :initial-element #\─) "┘") :sgr edge)))))

(defun windows (snapshot event)
  (declare (ignore event))
  (let ((focus (ekko/extensions:value snapshot :focus))
        (mode (ekko/extensions:value snapshot :mode)))
    (list (ekko/extensions:action :decorate :spans
      (append
        (loop for pane in (ekko/extensions:value snapshot :panes)
              when (getf pane :visible)
              append
          (destructuring-bind (x y width height) (getf pane :outer-rect)
            (let* ((id (getf pane :id)) (controls (>= width 12))
                   (attribute (if (eql id focus) 1 2))
                   (frame-sgr (list 0 attribute 38 5 (desktop-color id) 49))
                   (bar-sgr (list 0 attribute 38 5 (desktop-color id) 48 5 235))
                   (title (fit-text
                           (safe-text (pane-title pane))
                           (max 0 (- width (if controls 20 2)))))
                   (spans (frame-spans
                           (list :rect (list x y width height) :title title
                                 :focus (eql id focus) :mode mode
                                 :sgr frame-sgr))))
              (setf spans
                (loop for span in spans collect
                  (append span (list :drag
                      (cond ((= (getf span :y) y) :move)
                            ((= (getf span :y) (+ y height -1)) :bottom)
                            ((= (getf span :x) x) :left) (t :right))))))
              (setf (getf (first spans) :action) (list :focus :pane id)
                    (getf (first spans) :sgr) bar-sgr
                    (getf (first spans) :hover-sgr)
                    (list 0 1 38 5 16 48 5 237))
              (let ((arguments (list (write-to-string id))))
                (mapcar (lambda (span) (append span (list :pane id :context-command "desktop-window-menu" :arguments arguments)))
                  (append spans
                (when (getf pane :floating)
                  (list (list :x (1+ x) :y y :text "─" :sgr bar-sgr :drag :top)
                        (list :x x :y (+ y height -1) :text "└" :sgr bar-sgr :drag :bottom-left)
                        (list :x (+ x width -1) :y (+ y height -1) :text "┘" :sgr bar-sgr :drag :bottom-right)))
                (loop for edge in (remove-duplicates (list x (+ x width -1)))
                      collect (list :x edge :y y :text "│"
                                    :sgr bar-sgr
                                    :drag (if (getf pane :floating)
                                              (if (= edge x) :top-left :top-right) :top)
                                    :action (list :focus :pane id)))
                (when controls
                  (loop for (glyph color action) in '((" _ " 221 :minimize)
                                                      (" □ " 114 :zoom)
                                                      (" × " 203 :close))
                        for offset from (- width 10) by 3
                        collect (list :x (+ x offset) :y y :text glyph
                                      :sgr (list 0 1 38 5 color 48 5 235)
                                      :hover-sgr (list 0 1 38 5 16 48 5 color)
                                      :action (list action :pane id))))))))))
        (unless (find-if (lambda (pane) (getf pane :visible)) (ekko/extensions:value snapshot :panes))
          (backdrop snapshot)))))))

(defun dock (snapshot event &optional (hints *hints*))
  (declare (ignore event))
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (width (getf viewport :cols)) (y (1- (getf viewport :rows)))
         (panes (ekko/extensions:value snapshot :panes))
         (focus (ekko/extensions:value snapshot :focus))
         (mode (or (ekko/extensions:value snapshot :mode) :normal))
         (session (ekko/extensions:value snapshot :session))
         (base '(0 38 5 250 48 5 235))
         (left (list (list " EKKO " '(0 1 38 5 154 48 5 235))
                     (list (format nil "~A │ " (fit-text (safe-text session) 16)) base)))
         (right (if (eq mode :normal)
                    (list (list (format nil " ~A " (clock-text (ekko/extensions:value snapshot :time)))
                                '(0 1 38 5 250 48 5 235)))
                    (cons (list (format nil "  ~A " (string-upcase (symbol-name mode)))
                                '(0 1 38 5 221 48 5 235))
                          (loop for (key label) in (cdr (assoc mode hints))
                                collect (list (format nil " ~A ~A " key label) base)))))
         (left-used (loop for segment in left sum (ekko/extensions:display-width (first segment))))
         (right-used (loop for segment in right sum (ekko/extensions:display-width (first segment))))
         (space (max 0 (- width left-used right-used)))
         (tile (max 3 (min 24 (floor (max 0 (- space 1 (max 0 (1- (length panes)))))
                                     (max 1 (length panes))))))
         (entries nil) (overflow nil))
    (labels ((width-of (n) (+ (* n tile) (max 0 (1- n))))
             (entry (pane)
               (let* ((id (getf pane :id)) (minimized (getf pane :minimized))
                      (marker (cond ((eql id focus) "▸") ((getf pane :activity) "●") (minimized "_") (t "·")))
                      (title (fit-text (safe-text (pane-title pane)) (max 0 (- tile 3)))))
                 (list :x 0 :y y :text (format nil " ~A~A " marker title)
                       :sgr (if minimized (list 0 38 5 (desktop-color id) 48 5 239)
                                (list 0 1 38 5 16 48 5 (desktop-color id)))
                       :hover-sgr (if minimized (list 0 38 5 (desktop-color id) 48 5 245)
                                      (list 0 1 38 5 16 48 5 238))
                       :action (list (cond (minimized :restore) ((eql id focus) :minimize) (t :focus)) :pane id)
                       :arguments (list (write-to-string id))
                       :context-command "desktop-window-menu"
                       :wheel-command "desktop-cycle"
                       :middle-command "desktop-close"))))
      (let ((fit (loop for n downfrom (length panes) to 0 when (<= (width-of n) space) return n)))
        (if (= fit (length panes))
            (setf entries (mapcar #'entry panes))
            (let* ((limited (loop for n downfrom (min fit (1- (length panes))) to 0
                                  when (<= (width-of n) (max 0 (- space 6))) return n))
                   (shown (or limited 0)) (hidden (- (length panes) shown)))
              (setf entries (mapcar #'entry (subseq panes 0 shown)))
              (when (and (plusp hidden)
                         (<= (+ (width-of shown) 3 (length (princ-to-string hidden))) space))
                (setf overflow (format nil " +~D " hidden)))))))
    (when (plusp (third (getf viewport :insets)))
      (let ((spans (list (list :x 0 :y y :text (make-string width :initial-element #\Space)
                               :sgr base :context-command "desktop-taskbar-menu")))
            (x 0))
        (labels ((place (span)
                   (let ((text (fit-text (getf span :text) (max 0 (- width x)))))
                     (when (plusp (length text))
                       (setf (getf span :text) text (getf span :x) x)
                       (push span spans)
                       (incf x (ekko/extensions:display-width text))))))
          (dolist (segment left)
            (place (list :x 0 :y y :text (first segment) :sgr (second segment)
                         :context-command "desktop-taskbar-menu")))
          (let ((first-entry t))
            (dolist (span entries)
              (when (and (not first-entry) (< (1+ x) width)) (incf x))
              (setf first-entry nil)
              (place span))
            (when overflow
              (place (list :x 0 :y y :text overflow :sgr '(0 1 38 5 221 48 5 235)
                           :hover-sgr '(0 1 38 5 16 48 5 221)
                           :command "desktop-window-list"))))
          (setf x (max x (- width right-used)))
          (dolist (segment right)
            (place (list :x 0 :y y :text (first segment) :sgr (second segment)
                         :context-command "desktop-taskbar-menu"))))
        (list (ekko/extensions:action :decorate :spans (nreverse spans)))))))

(defun hook (snapshot event)
  (list (ekko/extensions:action :decorate :spans
          (append (getf (rest (first (windows snapshot event))) :spans)
                  (getf (rest (first (dock snapshot event))) :spans)))))

(defun install-controls (owner)
  (ekko/extensions:set-option :component owner :name :window-animation-ms :value 160)
  (ekko/extensions:register-command :component owner :name "desktop-toggle-floating"
    :handler (lambda (snapshot event) (declare (ignore event))
               (let ((pane (find (ekko/extensions:value snapshot :focus)
                                 (ekko/extensions:value snapshot :panes)
                                 :key (lambda (p) (getf p :id)))))
                 (when pane
                   (list (ekko/extensions:action (if (getf pane :floating) :tile :float)
                                                :pane (getf pane :id))
                         (ekko/extensions:action :set-keymap :name :normal))))))
  (ekko/extensions:bind-key :component owner :map :pane :key "t" :command "desktop-toggle-floating")
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
  (ekko/extensions:bind-key :component owner :map :pane :key "m" :command "desktop-minimize")
  (ekko/extensions:register-command :component owner :name "desktop-window-list"
    :handler (lambda (snapshot event) (declare (ignore event))
               (window-list-menu snapshot)))
  (ekko/extensions:register-command :component owner :name "desktop-switcher"
    :handler (lambda (snapshot event) (declare (ignore event))
               (window-list-menu snapshot)))
  (ekko/extensions:bind-key :component owner :map :normal :key "M-Tab" :command "desktop-switcher")
  (ekko/extensions:register-command :component owner :name "desktop-cycle"
    :handler (lambda (snapshot event)
               (let* ((panes (ekko/extensions:value snapshot :panes))
                      (focus (position (ekko/extensions:value snapshot :focus) panes
                                       :key (lambda (pane) (getf pane :id))))
                      (direction (or (getf event :direction) 1)))
                 (when (and focus panes)
                   (let ((pane (nth (mod (+ focus direction) (length panes)) panes)))
                     (list (ekko/extensions:action :focus :pane (getf pane :id))))))))
  (ekko/extensions:register-command :component owner :name "desktop-close"
    :handler (lambda (snapshot event)
               (let* ((argument (first (getf event :arguments)))
                      (id (and argument (parse-integer argument :junk-allowed t)))
                      (pane (find id (ekko/extensions:value snapshot :panes)
                                  :key (lambda (pane) (getf pane :id)))))
                 (unless pane (error "Window no longer exists"))
                 (list (ekko/extensions:action :close :pane id)))))
  (ekko/extensions:register-command :component owner :name "desktop-focus-slot"
    :handler (lambda (snapshot event)
               (let* ((argument (first (getf event :arguments)))
                      (slot (and argument (parse-integer argument :junk-allowed t)))
                      (panes (ekko/extensions:value snapshot :panes))
                      (pane (and slot (<= 1 slot) (nth (1- slot) panes))))
                 (when pane (list (ekko/extensions:action :focus :pane (getf pane :id)))))))
  (loop for index from 1 to 9 do
    (ekko/extensions:bind-key :component owner :map :normal :key (format nil "Super-~D" index)
                              :command "desktop-focus-slot"
                              :arguments (list (write-to-string index)))))
