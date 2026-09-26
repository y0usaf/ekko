;; Shared desktop styling, compiled into defaults and loadable by bare profiles.
;; All interaction uses ordinary, owner-scoped decoration actions.
(defpackage #:ekko/desktop
  (:use #:cl)
  (:export #:windows #:dock #:hook #:install-controls #:*hints*
           #:*theme* #:theme #:sgr #:clip #:panel #:menu-spans #:status-mark #:pane-title))
(in-package #:ekko/desktop)

(defparameter *hints*
  '((:normal) (:locked ("Ctrl-g" "Unlock"))
    (:pane ("r/d/n" "New pane") ("Tab" "Cycle/restore") ("b" "Notifications") ("h/j/k/l" "Focus")
           ("m" "Minimize") ("f" "Maximize") ("t" "Float/tile") ("c" "Rename") ("x" "Close") ("Esc" "Normal"))
    (:move ("h/j/k/l" "Move pane") ("n/Tab" "Next") ("p" "Previous") ("Esc" "Normal"))
    (:rename ("Type" "Rename pane") ("Enter" "Save") ("Esc" "Cancel"))
    (:session ("d" "Detach") ("Ctrl-q" "Quit") ("Esc" "Normal"))))

;; The look is data. Colours are 256-colour indices; a profile restyles every
;; panel, menu, toast and taskbar entry by rebinding or editing *theme*.
(defparameter *theme*
  '(:surface 235 :raised 237 :selection 238 :border 241 :text 252 :muted 245 :faint 241
    :attention 221 :danger 203 :ok 114 :on-accent 16
    :accents (154 117 213 221 114 209)
    :corners ("╭" "╮" "╰" "╯") :horizontal #\─ :vertical "│" :tees ("├" "┤") :ellipsis "…"
    :marks (:focus "▸" :attention "!" :busy "◐" :activity "●" :minimized "–" :idle "·")
    :bell "🔔" :toast-seconds 6 :toast-width 48
    ;; Depth. The desktop itself is the core :ground option, which also shows
    ;; through every window; :shadow is a colour cast by floating windows and
    ;; panels, the things above the tiled plane; a
    ;; :filled titlebar paints the focused window's title strip in its accent;
    ;; :attention-style :mark colours only the taskbar mark, :entry the whole entry.
    :shadow 233 :titlebar :plain :attention-style :entry
    ;; Roles. Each names what it colours and defaults to a token above; nil
    ;; means "the window's accent". A theme restyles one surface by setting its
    ;; role, and a colour anywhere may be an index or an (r g b) triple.
    ;; :frame-style :solid fills borders with the frame colour instead of
    ;; drawing lines on the ground.
    :frame-style :line :title-align :center
    :frame-active nil :frame-inactive :border
    :title-active nil :title-inactive :raised :title-text :on-accent :title-inactive-text :muted
    :control-bg nil :close-bg nil
    :bar-bg :surface :bar-text :muted :start nil :tray-bg :bar-bg :tray-text :text
    :entry-bg :raised :entry-text nil :entry-active nil :entry-active-text :on-accent :entry-hover nil
    :entry-minimized :bar-bg :entry-minimized-text :faint
    :panel-bg :surface :panel-text :text :panel-muted :muted :panel-border :border
    :highlight nil :highlight-text :on-accent
    :toast-bg :panel-bg :toast-text :panel-text :toast-frame :attention))

(defun theme (key) (getf *theme* key))

(defun color (value)
  "Resolve a theme keyword, following aliases, to an index or (r g b)."
  (loop while (keywordp value) do (setf value (theme value)))
  value)

(defun layer (code value)
  "SGR parameters for colour VALUE: CODE is 38 for foreground, 48 for background."
  (let ((c (color value)))
    (if (listp c) (list code 2 (first c) (second c) (third c)) (list code 5 c))))

(defun sgr (fg &optional (bg :surface) bold)
  (append (list 0) (when bold (list 1)) (layer 38 fg) (layer 48 bg)))

(defun ink (fg &optional bold)
  "Text colour FG on the panel background."
  (sgr fg :panel-bg bold))

(defun role (key id)
  "Role KEY's colour, or window ID's accent when the role is nil."
  (or (theme key) (desktop-color id)))

(defun mark (key) (getf (theme :marks) key))

(defun desktop-color (id)
  (let ((accents (theme :accents))) (nth (mod (1- id) (length accents)) accents)))

(defun safe-text (text)
  (map 'string (lambda (c) (if (or (< (char-code c) 32) (<= 127 (char-code c) 159)) #\Space c))
       (or text "")))

(defun fit-text (text width)
  (with-output-to-string (out)
    (loop with used = 0 for c across text
          for size = (ekko/extensions:display-width c)
          while (<= (+ used size) width)
          do (write-char c out) (incf used size))))

(defun clip (text width)
  "TEXT in at most WIDTH cells, ending in the theme's ellipsis when cut."
  (if (<= (ekko/extensions:display-width text) width)
      text
      (concatenate 'string (fit-text text (max 0 (1- width))) (if (plusp width) (theme :ellipsis) ""))))

(defun padded (text width &optional (fill #\Space))
  (let ((text (clip text width)))
    (concatenate 'string text (make-string (max 0 (- width (ekko/extensions:display-width text)))
                                           :initial-element fill))))

(defun busy-glyph-p (char)
  "Spinner glyphs programs put before their title while they work."
  (or (<= #x2800 (char-code char) #x28FF) (find char "◐◑◒◓◴◵◶◷")))

(defun title-parts (pane)
  "The pane's display title, and whether its program marked it busy. A leading
status glyph and space, as coding agents write them, becomes the busy flag."
  (let* ((name (getf pane :name))
         (raw (cond ((and name (plusp (length name))) name)
                    ((getf pane :terminal-title)
                     (string-trim '(#\Space #\Tab #\Newline #\Return) (getf pane :terminal-title)))
                    ((eq (getf pane :launch-kind) :command) (format nil "~{~A~^ ~}" (getf pane :argv)))
                    (t (format nil "Pane #~D" (getf pane :id)))))
         (glyph (and (> (length raw) 2) (char= (char raw 1) #\Space)
                     (not (alphanumericp (char raw 0))) (not (find (char raw 0) "~/.([<$#-"))
                     (char raw 0))))
    (values (safe-text (if glyph (subseq raw 2) raw)) (and glyph (busy-glyph-p glyph)))))

(defun pane-title (pane) (values (title-parts pane)))

(defun border-glyph () (if (eq (theme :frame-style) :solid) " " (theme :vertical)))

(defun frame-spans (pane)
  (destructuring-bind (x y width height) (getf pane :rect)
    (let* ((title (clip (getf pane :title) (max 0 (- width 2))))
           (padding (- width (ekko/extensions:display-width title)))
           (left (if (eq (theme :title-align) :left) (min 2 padding) (floor padding 2)))
           (sgr (getf pane :sgr)) (corners (theme :corners)) (solid (eq (theme :frame-style) :solid))
           (header (concatenate 'string (make-string left :initial-element #\Space) title
                                (make-string (- padding left) :initial-element #\Space))))
      (append (list (list :x x :y y :text header :sgr sgr))
        (when (> height 2)
          (loop for edge in (remove-duplicates (list x (+ x width -1)))
                collect (list :x edge :y (1+ y) :text (border-glyph) :sgr sgr :rows (- height 2))))
        (when (> height 1)
          (list (list :x x :y (+ y height -1)
                      :text (cond (solid (make-string width :initial-element #\Space))
                                  ((= width 1) (theme :vertical))
                                  (t (rule (third corners) (fourth corners) width)))
                      :sgr sgr)))))))

(defun status-mark (pane focus unread)
  "One status vocabulary for the taskbar and the switcher: (values glyph colour)."
  (let ((id (getf pane :id)))
    (cond ((eql id focus) (values (mark :focus) (desktop-color id)))
          ((find id unread :key (lambda (n) (getf n :pane))) (values (mark :attention) :attention))
          ((nth-value 1 (title-parts pane)) (values (mark :busy) (desktop-color id)))
          ((getf pane :activity) (values (mark :activity) (desktop-color id)))
          ((getf pane :minimized) (values (mark :minimized) :faint))
          (t (values (mark :idle) :faint)))))

(defun rule (left right width)
  "A horizontal border WIDTH cells wide."
  (concatenate 'string left
               (make-string (max 0 (- width (ekko/extensions:display-width left) (ekko/extensions:display-width right)))
                            :initial-element (theme :horizontal))
               right))

(defun panel (rows &key title hints (width 40) (accent :text) (frame :panel-border) (bg :panel-bg) enter overlay)
  "Spans for a bordered panel at (0 0), and its height. Each row is a plist:
:text, optional :lead and :right strings, :sgr, :key, and a control (:command
with :arguments, or :action); (:separator t) draws a divider. TITLE sits in the
top border and HINTS in the bottom one, both inset in their own colours. Only
row content is a control, so the frame keeps its colour under the pointer."
  (let* ((edge (sgr frame bg)) (hover (sgr :highlight-text (or (theme :highlight) accent) t))
         (corners (theme :corners)) (y 0) (spans nil)
         (common (append (when enter (list :enter enter)) (when overlay (list :overlay t)))))
    (flet ((emit (x text style &rest more)
             (push (append (list :x x :y y :text text :sgr style) more common) spans)))
      (emit 0 (rule (first corners) (second corners) width) edge)
      (when title (emit 2 (concatenate 'string " " (clip title (- width 6)) " ") (sgr :panel-text bg t)))
      (incf y)
      (dolist (row rows)
        (if (getf row :separator)
            (emit 0 (rule (first (theme :tees)) (second (theme :tees)) width) edge)
            (let* ((lead (getf row :lead)) (right (or (getf row :right) ""))
                   (inner (- width 4 (if lead (1+ (ekko/extensions:display-width lead)) 0)
                             (ekko/extensions:display-width right)))
                   (text (concatenate 'string " " (if lead (concatenate 'string lead " ") "")
                                      (padded (getf row :text) inner) right " "))
                   (control (loop for key in '(:command :arguments :action :key)
                                  when (getf row key) append (list key (getf row key)))))
              (emit 0 (theme :vertical) edge)
              (apply #'emit 1 text (or (getf row :sgr) (sgr :panel-text bg))
                     (when (or (getf row :command) (getf row :action)) (list* :hover-sgr hover control)))
              (emit (1- width) (theme :vertical) edge)))
        (incf y))
      (emit 0 (rule (third corners) (fourth corners) width) edge)
      (when hints (emit 2 (concatenate 'string " " (clip hints (- width 6)) " ") (sgr :panel-muted bg)))
      (incf y)
      ;; Panels float above windows, so they cast the theme's shadow too.
      (let ((shadow (theme :shadow)))
        (when shadow
          (push (append (list :x width :y 1 :rows (1- y) :text " " :sgr (sgr shadow shadow)) common) spans)
          (push (append (list :x 1 :y y :text (make-string width :initial-element #\Space) :sgr (sgr shadow shadow))
                        common)
                spans)))
      (values (nreverse spans) (if (theme :shadow) (1+ y) y)))))

(defun menu-spans (items color &optional title)
  "ITEMS are (label action command arguments); the menu takes the owner's COLOR."
  (let ((width (min 40 (max 24 (+ 6 (loop for item in items maximize (ekko/extensions:display-width (first item))))))))
    (values (panel (loop for (label action command arguments) in items
                         collect (append (list :text label)
                                         (when action (list :action action))
                                         (when command (list :command command :arguments arguments))))
                   :title title :width width :accent color :enter (list "menu" 0 -1)))))

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
      (desktop-color id) (clip (pane-title pane) 24))))

(defun clock-text (time)
  (multiple-value-bind (second minute hour) (decode-universal-time time)
    (declare (ignore second))
    (format nil "~2,'0D:~2,'0D" hour minute)))

(defun unread (snapshot)
  (remove-if (lambda (n) (getf n :read)) (ekko/extensions:value snapshot :notifications)))

(defun window-list-menu (snapshot)
  "Windows, most recently used first; 1-9 pick directly."
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (focus (ekko/extensions:value snapshot :focus)) (unread (unread snapshot))
         (panes (sort (copy-list (ekko/extensions:value snapshot :panes))
                      #'> :key (lambda (pane) (getf pane :activation-order))))
         (width (min (getf viewport :cols) 64
                     (max 36 (+ 14 (loop for pane in panes maximize (ekko/extensions:display-width (pane-title pane)))))))
         (rows (loop for pane in panes for index from 1
                     collect (multiple-value-bind (glyph tone) (status-mark pane focus unread)
                               (declare (ignore tone))
                               (append (list :lead (format nil "~A ~A" (if (<= index 9) index " ") glyph)
                                             :text (pane-title pane)
                                             :right (if (getf pane :minimized) "minimized" "")
                                             :sgr (if (getf pane :minimized) (ink :panel-muted) (ink :panel-text))
                                             :action (list (if (getf pane :minimized) :restore :focus) :pane (getf pane :id)))
                                       (when (<= index 9) (list :key (princ-to-string index))))))))
    (multiple-value-bind (spans height)
        (panel rows :title "Windows" :hints "1-9 · ⏎ open · esc" :width width
                    :accent (desktop-color (or focus 1)) :enter (list "switcher" 0 -1))
      (list (ekko/extensions:action :show-menu
              ':x (max 0 (floor (- (getf viewport :cols) width) 2))
              :y (max 0 (floor (- (getf viewport :rows) height) 2))
              :spans spans)))))

(defun notification-window (n snapshot)
  "The sending window's title: it says which session wants attention."
  (let ((pane (find (getf n :pane) (ekko/extensions:value snapshot :panes) :key (lambda (p) (getf p :id)))))
    (if pane (pane-title pane) "Closed window")))

(defun notification-message (n)
  (let ((title (getf n :title)) (body (getf n :body)))
    (safe-text
     (cond ((getf n :bell) (if (> (getf n :count) 1) (format nil "Bell ×~D" (getf n :count)) "Bell"))
           ((and (plusp (length title)) (plusp (length body))) (format nil "~A: ~A" title body))
           ((plusp (length body)) body)
           (t title)))))

(defun age-text (seconds)
  (cond ((< seconds 60) "now") ((< seconds 3600) (format nil "~Dm" (floor seconds 60)))
        ((< seconds 86400) (format nil "~Dh" (floor seconds 3600))) (t (format nil "~Dd" (floor seconds 86400)))))

(defun toast (snapshot)
  "The newest unread notification, briefly, sliding in above the taskbar's right end."
  (let* ((n (first (unread snapshot)))
         (viewport (ekko/extensions:value snapshot :viewport))
         (cols (getf viewport :cols)) (rows (getf viewport :rows))
         (width (min (theme :toast-width) cols)))
    (when (and n (< (- (ekko/extensions:value snapshot :time) (getf n :time)) (theme :toast-seconds)) (>= rows 5))
      (let ((open (list :command "desktop-notification-open" :arguments (list (write-to-string (getf n :id))))))
        (multiple-value-bind (spans height)
            (panel (list (append (list :text (notification-message n) :sgr (sgr :toast-text :toast-bg)) open))
                   :title (concatenate 'string (theme :bell) " " (clip (notification-window n snapshot) (- width 12)))
                   :width width :accent :attention :frame :toast-frame :bg :toast-bg :overlay t
                   :enter (list (format nil "toast-~D" (getf n :id)) width 0))
          ;; Sit on the taskbar, shadow included; the panel is one column wider with a shadow.
          (let ((x (- cols width (if (theme :shadow) 1 0))) (y (- rows 1 height)))
            (loop for span in spans
                  collect (append (list :x (+ (getf span :x) x) :y (+ (getf span :y) y))
                                  (loop for (key value) on span by #'cddr unless (member key '(:x :y)) append (list key value))
                                  (unless (getf span :command) open)))))))))

(defun notification-menu (snapshot)
  "Newest first; clicking an entry opens its window. Opening the centre reads them all."
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (now (ekko/extensions:value snapshot :time))
         (notifications (ekko/extensions:value snapshot :notifications))
         (width (min 52 (getf viewport :cols)))
         (rows (if notifications
                   (append
                    (loop for n in notifications
                          for open = (list :command "desktop-notification-open"
                                           :arguments (list (write-to-string (getf n :id))))
                          append (list (append (list :lead (if (getf n :read) " " (mark :activity))
                                                     :text (notification-window n snapshot)
                                                     :right (age-text (max 0 (- now (getf n :time))))
                                                     :sgr (if (getf n :read) (ink :panel-muted) (ink :panel-text t)))
                                               open)
                                       (append (list :lead " " :text (notification-message n) :sgr (ink :panel-muted)) open)))
                    (list (list :separator t)
                          (list :lead " " :text "Clear all" :sgr (ink :panel-muted) :command "desktop-notifications-clear")))
                   (list (list :text "No notifications" :sgr (ink :panel-muted))))))
    (multiple-value-bind (spans height)
        (panel rows :title "Notifications" :hints "⏎ open · esc" :width width :accent :attention
                    :enter (list "notifications" 0 2))
      (list (ekko/extensions:action :show-menu ':x (max 0 (- (getf viewport :cols) width))
                                    :y (max 0 (- (getf viewport :rows) 1 height)) :spans spans)
            (ekko/extensions:action :notifications ':op :read-all)))))

(defun backdrop (snapshot)
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (cols (getf viewport :cols)) (rows (getf viewport :rows)) (width (min cols 36)))
    (multiple-value-bind (spans height)
        (panel (list (list :text (clock-text (ekko/extensions:value snapshot :time)) :sgr (ink :panel-text t))
                     (list :text (format nil "session ~A" (safe-text (ekko/extensions:value snapshot :session)))
                           :sgr (ink :panel-muted))
                     (list :separator t)
                     (list :lead "Ctrl-p n  " :text "new window" :sgr (ink :panel-muted))
                     (list :lead "Ctrl-p Tab" :text "restore window" :sgr (ink :panel-muted)))
               :title "EKKO" :width width)
      (let ((x (max 0 (floor (- cols width) 2))) (y (max 0 (floor (- (max 1 (1- rows)) height) 2))))
        (loop for span in spans
              do (incf (getf span :x) x) (incf (getf span :y) y)
              collect span)))))

(defun surface (snapshot)
  "Shadows under floating windows, the things above the tiled plane. Decorations
never cover application content, so these land only on the ground around them."
  (let ((shadow (theme :shadow)))
    (when shadow
      (loop for pane in (ekko/extensions:value snapshot :panes)
            when (and (getf pane :visible) (getf pane :floating))
              append (destructuring-bind (x y width h) (getf pane :outer-rect)
                       (list (list :x (+ x width) :y (1+ y) :rows h :text " " :sgr (sgr shadow shadow)
                                   :follows (getf pane :id))
                             (list :x (1+ x) :y (+ y h) :text (make-string width :initial-element #\Space)
                                   :sgr (sgr shadow shadow) :follows (getf pane :id))))))))

(defun windows (snapshot event)
  (declare (ignore event))
  (let ((focus (ekko/extensions:value snapshot :focus))
        (mode (ekko/extensions:value snapshot :mode))
        (viewport (ekko/extensions:value snapshot :viewport)))
    (list (ekko/extensions:action :decorate ':spans
      (ekko/extensions:clip-decorations
       (append
        (surface snapshot)
        (loop for pane in (ekko/extensions:value snapshot :panes)
              when (getf pane :visible)
              append
          (destructuring-bind (x y width height) (getf pane :outer-rect)
            (let* ((id (getf pane :id)) (controls (>= width 12))
                   (focused (eql id focus))
                   ;; Only the focused window wears its accent; the rest recede.
                   (frame (if focused (role :frame-active id) :frame-inactive))
                   (frame-sgr (if (eq (theme :frame-style) :solid)
                                  (sgr frame frame)
                                  ;; No background: line borders sit on the ground.
                                  (append (list 0) (when focused (list 1)) (layer 38 frame))))
                   (filled (eq (theme :titlebar) :filled))
                   (title-bg (if focused (role :title-active id) :title-inactive))
                   (bar-sgr (cond (filled (sgr (if focused :title-text :title-inactive-text) title-bg t))
                                  (focused (sgr (desktop-color id) :surface t))
                                  (t (sgr :muted :surface))))
                   (title (clip (pane-title pane) (max 0 (- width (if controls 20 2)))))
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
                    (if filled bar-sgr (sgr (if focused (desktop-color id) :text) :raised t)))
              (let ((arguments (list (write-to-string id))))
                (mapcar (lambda (span) (append span (list :pane id :context-command "desktop-window-menu" :arguments arguments)))
                  (append spans
                (when (getf pane :floating)
                  (let ((solid (eq (theme :frame-style) :solid)))
                    (list (list :x (1+ x) :y y :text (if (or solid filled) " " (string (theme :horizontal))) :sgr bar-sgr :drag :top)
                          (list :x x :y (+ y height -1) :text (if solid " " (third (theme :corners))) :sgr frame-sgr
                                :drag :bottom-left)
                          (list :x (+ x width -1) :y (+ y height -1) :text (if solid " " (fourth (theme :corners)))
                                :sgr frame-sgr :drag :bottom-right))))
                (loop for edge in (remove-duplicates (list x (+ x width -1)))
                      collect (list :x edge :y y :text (border-glyph)
                                    :sgr frame-sgr
                                    :drag (if (getf pane :floating)
                                              (if (= edge x) :top-left :top-right) :top)
                                    :action (list :focus :pane id)))
                (when controls
                  (loop for (glyph tone action) in '((" _ " :attention :minimize)
                                                     (" □ " :ok :zoom)
                                                     (" × " :danger :close))
                        for offset from (- width 10) by 3
                        collect (list :x (+ x offset) :y y :text glyph
                                      :sgr (cond ((and focused filled)
                                                  (sgr :title-text (or (theme (if (eq action :close) :close-bg :control-bg)) title-bg) t))
                                                 (focused (sgr tone :surface t))
                                                 (filled (sgr :title-inactive-text title-bg))
                                                 (t (sgr :faint :surface)))
                                      :hover-sgr (sgr :on-accent tone t)
                                      :action (list action :pane id))))))))))
        (unless (find-if (lambda (pane) (getf pane :visible)) (ekko/extensions:value snapshot :panes))
          (backdrop snapshot))
        (toast snapshot))
       (getf viewport :cols) (getf viewport :rows))))))

(defun dock (snapshot event &optional (hints *hints*))
  (declare (ignore event))
  (let* ((viewport (ekko/extensions:value snapshot :viewport))
         (width (getf viewport :cols)) (y (1- (getf viewport :rows)))
         (panes (ekko/extensions:value snapshot :panes))
         (focus (ekko/extensions:value snapshot :focus))
         (mode (or (ekko/extensions:value snapshot :mode) :normal))
         (session (ekko/extensions:value snapshot :session))
         (base (sgr :bar-text :bar-bg))
         (start (theme :start))
         (left (if start
                   (destructuring-bind (text fg bg) start
                     (list (list text (sgr fg bg t)
                                 (list :command "desktop-start-menu" :hover-sgr (sgr fg bg t)))
                           (list " " base)))
                   (list (list " EKKO " (sgr (first (theme :accents)) :bar-bg t))
                         (list (format nil "~A │ " (clip (safe-text session) 16)) base))))
         (unread (unread snapshot))
         (bell (list (if unread (format nil " ~A ~D " (theme :bell) (length unread)) (format nil " ~A " (theme :bell)))
                     (if unread (sgr :on-accent :attention t) (sgr :tray-text :tray-bg))
                     (list :command "desktop-notifications" :hover-sgr (sgr :on-accent :text t))))
         (right (cons bell (if (eq mode :normal)
                    (list (list (format nil " ~A " (clock-text (ekko/extensions:value snapshot :time)))
                                (sgr :tray-text :tray-bg t)))
                    (cons (list (format nil "  ~A " (string-upcase (symbol-name mode)))
                                (sgr :attention :bar-bg t))
                          (loop for (key label) in (rest (assoc mode hints))
                                collect (list (format nil " ~A ~A " key label) base))))))
         (left-used (loop for segment in left sum (ekko/extensions:display-width (first segment))))
         (right-used (loop for segment in right sum (ekko/extensions:display-width (first segment))))
         (space (max 0 (- width left-used right-used)))
         (tile (max 3 (min 24 (floor (max 0 (- space 1 (max 0 (1- (length panes)))))
                                     (max 1 (length panes))))))
         (entries nil) (overflow nil))
    (labels ((width-of (n) (+ (* n tile) (max 0 (1- n))))
             (entry (pane)
               (let* ((id (getf pane :id)) (minimized (getf pane :minimized))
                      (title (clip (pane-title pane) (max 0 (- tile 4)))))
                 ;; Only the focused entry is filled. The mark and the title are
                 ;; separate spans so attention can colour either one.
                 (multiple-value-bind (glyph tone) (status-mark pane focus unread)
                   (let* ((attention (eq tone :attention))
                          (body (cond ((eql id focus) (sgr :entry-active-text (role :entry-active id) t))
                                      ((and attention (eq (theme :attention-style) :entry)) (sgr :attention :entry-bg t))
                                      (minimized (sgr :entry-minimized-text :entry-minimized))
                                      (t (sgr (role :entry-text id) :entry-bg))))
                          (bg (cond ((eql id focus) (role :entry-active id)) (minimized :entry-minimized) (t :entry-bg)))
                          (shared (list :hover-sgr (sgr :entry-active-text (role :entry-hover id) t)
                                        :action (list (cond (minimized :restore) ((eql id focus) :minimize) (t :focus)) :pane id)
                                        :arguments (list (write-to-string id))
                                        :context-command "desktop-window-menu"
                                        :wheel-command "desktop-cycle"
                                        :middle-command "desktop-close")))
                     (list (list* :x 0 :y y :text (format nil " ~A" glyph)
                                  :sgr (if (and attention (eq (theme :attention-style) :mark) (not (eql id focus)))
                                           (sgr :attention bg t) body)
                                  shared)
                           (list* :x 0 :y y :text (format nil " ~A " title) :sgr body shared)))))))
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
            (dolist (entry entries)
              (when (and (not first-entry) (< (1+ x) width)) (incf x))
              (setf first-entry nil)
              (mapc #'place entry))
            (when overflow
              (place (list :x 0 :y y :text overflow :sgr (sgr :attention :bar-bg t)
                           :hover-sgr (sgr :on-accent :attention t)
                           :command "desktop-window-list"))))
          (setf x (max x (- width right-used)))
          (dolist (segment right)
            (place (append (list :x 0 :y y :text (first segment) :sgr (second segment))
                           (or (third segment) (list :context-command "desktop-taskbar-menu"))))))
        (list (ekko/extensions:action :decorate ':spans (nreverse spans)))))))

(defun hook (snapshot event)
  (list (ekko/extensions:action :decorate ':spans
          (append (getf (rest (first (windows snapshot event))) :spans)
                  (getf (rest (first (dock snapshot event))) :spans)))))

(defun install-controls (owner)
  (ekko/extensions:set-option :component owner :name ':window-animation-ms :value 160)
  (ekko/extensions:register-command :component owner :name "desktop-toggle-floating"
    :handler (lambda (snapshot event) (declare (ignore event))
               (let ((pane (find (ekko/extensions:value snapshot :focus)
                                 (ekko/extensions:value snapshot :panes)
                                 :key (lambda (p) (getf p :id)))))
                 (when pane
                   (list (ekko/extensions:action (if (getf pane :floating) :tile :float)
                                                :pane (getf pane :id))
                         (ekko/extensions:action :set-keymap ':name :normal))))))
  (ekko/extensions:bind-key :component owner :map ':pane :key "t" :command "desktop-toggle-floating")
  (ekko/extensions:register-command :component owner :name "desktop-window-menu"
    :handler (lambda (snapshot event)
               (let* ((id (parse-integer (first (getf event :arguments))))
                      (pane (find id (ekko/extensions:value snapshot :panes) :key (lambda (p) (getf p :id)))))
                 (unless pane (error "Window no longer exists"))
                 (list (ekko/extensions:action :show-menu ':x (getf event :x) :y (getf event :y)
                         :spans (window-menu pane (ekko/extensions:value snapshot :focus)
                                             (ekko/extensions:value snapshot :zoom)))))))
  (ekko/extensions:register-command :component owner :name "desktop-taskbar-menu"
    :handler (lambda (snapshot event)
               (let ((focus (ekko/extensions:value snapshot :focus)))
                 (list (ekko/extensions:action :show-menu ':x (getf event :x) :y (getf event :y)
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
                 (append (list (ekko/extensions:action :focus ':pane id))
                         (cl-user::zellij-pane-rename-enter-actions copy (string-downcase (string owner)))))))
  (ekko/extensions:register-command :component owner :name "desktop-next"
    :handler (lambda (snapshot event) (declare (ignore snapshot event))
               (list (ekko/extensions:action :focus-next))))
  (ekko/extensions:bind-key :component owner :map ':pane :key "Tab" :command "desktop-next")
  (ekko/extensions:register-command :component owner :name "desktop-minimize"
    :handler (lambda (snapshot event) (declare (ignore snapshot event))
               (list (ekko/extensions:action :minimize)
                     (ekko/extensions:action :set-keymap ':name :normal))))
  (ekko/extensions:bind-key :component owner :map ':pane :key "m" :command "desktop-minimize")
  (ekko/extensions:register-command :component owner :name "desktop-window-list"
    :handler (lambda (snapshot event) (declare (ignore event))
               (window-list-menu snapshot)))
  (ekko/extensions:register-command :component owner :name "desktop-switcher"
    :handler (lambda (snapshot event) (declare (ignore event))
               (window-list-menu snapshot)))
  (ekko/extensions:bind-key :component owner :map ':normal :key "M-Tab" :command "desktop-switcher")
  (ekko/extensions:register-command :component owner :name "desktop-start-menu"
    :handler (lambda (snapshot event) (declare (ignore event))
               (let ((focus (ekko/extensions:value snapshot :focus))
                     (viewport (ekko/extensions:value snapshot :viewport)))
                 (multiple-value-bind (spans height)
                     (panel (list (list :text "New window right" :action (list :split :pane focus :axis :columns))
                                  (list :text "New window down" :action (list :split :pane focus :axis :rows))
                                  (list :separator t)
                                  (list :text "Windows…" :command "desktop-window-list")
                                  (list :text "Notifications…" :command "desktop-notifications")
                                  (list :separator t)
                                  (list :text "Detach session" :command "session-detach"))
                            :title (safe-text (ekko/extensions:value snapshot :session)) :width 28
                            :accent (desktop-color (or focus 1)) :enter (list "start" 0 2))
                   (list (ekko/extensions:action :show-menu ':x 0 :y (max 0 (- (getf viewport :rows) 1 height))
                                                 :spans spans))))))
  (ekko/extensions:register-command :component owner :name "desktop-notifications"
    :handler (lambda (snapshot event) (declare (ignore event))
               (append (notification-menu snapshot)
                       (list (ekko/extensions:action :set-keymap ':name :normal)))))
  (ekko/extensions:bind-key :component owner :map ':pane :key "b" :command "desktop-notifications")
  (ekko/extensions:register-command :component owner :name "desktop-notifications-clear"
    :handler (lambda (snapshot event) (declare (ignore snapshot event))
               (list (ekko/extensions:action :notifications ':op :clear))))
  (ekko/extensions:register-command :component owner :name "desktop-notification-open"
    :handler (lambda (snapshot event)
               (let* ((id (parse-integer (first (getf event :arguments))))
                      (n (find id (ekko/extensions:value snapshot :notifications) :key (lambda (n) (getf n :id))))
                      (pane (and n (find (getf n :pane) (ekko/extensions:value snapshot :panes)
                                         :key (lambda (p) (getf p :id))))))
                 (unless pane (error "That window has closed"))
                 (list (ekko/extensions:action (if (getf pane :minimized) :restore :focus) ':pane (getf pane :id))
                       (ekko/extensions:action :notifications ':op :read :id id)))))
  (ekko/extensions:register-command :component owner :name "desktop-notification-latest"
    :handler (lambda (snapshot event) (declare (ignore event))
               (let* ((panes (ekko/extensions:value snapshot :panes))
                      (n (find-if (lambda (n) (find (getf n :pane) panes :key (lambda (p) (getf p :id))))
                                  (unread snapshot)))
                      (pane (and n (find (getf n :pane) panes :key (lambda (p) (getf p :id))))))
                 (when pane
                   (list (ekko/extensions:action (if (getf pane :minimized) :restore :focus) ':pane (getf pane :id))
                         (ekko/extensions:action :notifications ':op :read :id (getf n :id)))))))
  (ekko/extensions:bind-key :component owner :map ':normal :key "M-`" :command "desktop-notification-latest")
  (ekko/extensions:register-command :component owner :name "desktop-cycle"
    :handler (lambda (snapshot event)
               (let* ((panes (ekko/extensions:value snapshot :panes))
                      (focus (position (ekko/extensions:value snapshot :focus) panes
                                       :key (lambda (pane) (getf pane :id))))
                      (direction (or (getf event :direction) 1)))
                 (when (and focus panes)
                   (let ((pane (nth (mod (+ focus direction) (length panes)) panes)))
                     (list (ekko/extensions:action :focus ':pane (getf pane :id))))))))
  (ekko/extensions:register-command :component owner :name "desktop-close"
    :handler (lambda (snapshot event)
               (let* ((argument (first (getf event :arguments)))
                      (id (and argument (parse-integer argument :junk-allowed t)))
                      (pane (find id (ekko/extensions:value snapshot :panes)
                                  :key (lambda (pane) (getf pane :id)))))
                 (unless pane (error "Window no longer exists"))
                 (list (ekko/extensions:action :close ':pane id)))))
  (ekko/extensions:register-command :component owner :name "desktop-focus-slot"
    :handler (lambda (snapshot event)
               (let* ((argument (first (getf event :arguments)))
                      (slot (and argument (parse-integer argument :junk-allowed t)))
                      (panes (ekko/extensions:value snapshot :panes))
                      (pane (and slot (<= 1 slot) (nth (1- slot) panes))))
                 (when pane (list (ekko/extensions:action :focus ':pane (getf pane :id)))))))
  (loop for index from 1 to 9 do
    (ekko/extensions:bind-key :component owner :map ':normal :key (format nil "Super-~D" index)
                              :command "desktop-focus-slot"
                              :arguments (list (write-to-string index)))))
