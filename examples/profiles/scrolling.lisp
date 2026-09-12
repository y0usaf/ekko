;; A replaceable scrolling workspace, not a host layout mode.
;; EKKO_CONFIG=/absolute/path/to/scrolling.lisp ekko ...
;; Columns keep their width as panes are added. Focus moves a camera over a
;; strip of real, full-size terminals from every session in the shared daemon.
(defpackage #:ekko/scrolling
  (:use #:cl #:ekko/extensions))
(in-package #:ekko/scrolling)

(defun state (snapshot)
  (cdr (assoc "scrolling" (value snapshot :component-state) :test #'equal)))

(defun insets (width height requested)
  (let* ((top (min (first requested) (max 0 (1- height))))
         (left (min (fourth requested) (max 0 (1- width))))
         (bottom (min (third requested) (max 0 (- height top 1))))
         (right (min (second requested) (max 0 (- width left 1)))))
    (list top right bottom left)))

(defun columns (snapshot event)
  (declare (ignore event))
  (let* ((viewport (value snapshot :viewport)) (geometry (value snapshot :geometry))
         (padding (insets (getf viewport :cols) (getf viewport :rows)
                          (getf geometry :viewport-insets '(0 0 1 0))))
         (left (fourth padding)) (top (first padding))
         (width (- (getf viewport :cols) left (second padding)))
         (height (- (getf viewport :rows) top (third padding)))
         (panes (sort (remove-if (lambda (pane) (getf pane :minimized))
                                (copy-list (value snapshot :panes))) #'<
                      :key (lambda (pane) (getf pane :id))))
         (focus (value snapshot :focus)) (state (state snapshot))
         (column-width (getf state :column-width 64))
         (gap 2) (camera (first (getf (value snapshot :workspace) :camera '(0 0))))
         (placements
           (loop for pane in panes for index from 0
                 for x = (+ left (* index (+ column-width gap)))
                 for frame = (insets column-width height (getf geometry :pane-insets '(1 1 1 1)))
                 collect (list :pane (getf pane :id) :x (+ x (fourth frame))
                               :y (+ top (first frame))
                               :cols (- column-width (second frame) (fourth frame))
                               :rows (- height (first frame) (third frame))
                               :outer (list x top column-width height)))))
    (let ((focused (find focus placements :key (lambda (pane) (getf pane :pane)))))
      (when focused
        (let ((x (first (getf focused :outer))))
          (cond
            ;; Pan commands remain in control until focus changes.
            ((eql (getf state :pan-focus) focus) (setf camera (getf state :camera-x 0)))
            ((< x (+ camera left)) (setf camera (- x left)))
            ((> (+ x column-width) (+ camera left width))
             (setf camera (min (- x left) (- (+ x column-width) left width)))))))
      (list (action :place-panes :version 1 :placements placements
                    :camera (list (max 0 camera) 0))))))

(defun move-focus (snapshot delta)
  (let* ((panes (sort (copy-list (value snapshot :panes)) #'<
                      :key (lambda (pane) (getf pane :id))))
         (index (position (value snapshot :focus) panes
                          :key (lambda (pane) (getf pane :id)))))
    (when panes
      (list (action :focus :pane
                    (getf (nth (mod (+ (or index 0) delta) (length panes)) panes) :id))))))

(defun resize-columns (snapshot delta)
  (let ((next (copy-list (state snapshot))))
    (setf (getf next :column-width) (max 12 (min 300 (+ (getf next :column-width 64) delta))))
    (remf next :pan-focus)
    (list (action :set-state :value next))))

(defun pan (snapshot delta)
  (let ((next (copy-list (state snapshot))))
    (setf (getf next :camera-x)
          (max 0 (min 1000000 (+ (first (getf (value snapshot :workspace) :camera '(0 0))) delta)))
          (getf next :pan-focus) (value snapshot :focus))
    (list (action :set-state :value next))))

;; The desktop chrome draws frames for an on-screen tiled workspace. This
;; example replaces that policy and its bindings rather than asking the host
;; to recognize a special scrolling mode. The bare build can load it too.
(unregister-component :defaults)
(register-component :id :scrolling
                    :reads '(:panes :focus :component-state :workspace))
(register-layout-provider :component :scrolling :name "scrolling"
                          :reads '(:panes :focus :viewport :geometry :component-state :workspace)
                          :handler #'columns)
(set-option :component :scrolling :name :layout-provider :value "scrolling")
(set-option :component :scrolling :name :workspace-scope :value :all-panes)
(set-option :component :scrolling :name :pane-budget :value 128)
(set-option :component :scrolling :name :pane-insets :value '(0 0 0 0))
(set-option :component :scrolling :name :viewport-insets :value '(0 0 0 0))
(set-option :component :scrolling :name :prefix :value "C-a")

(dolist (spec '(("next-column" 1 "n") ("previous-column" -1 "p")))
  (destructuring-bind (name delta key) spec
    (register-command :component :scrolling :name name
                      :handler (lambda (snapshot event) (declare (ignore event))
                                 (move-focus snapshot delta)))
    (bind-key :component :scrolling :key key :command name)))
(dolist (spec '(("wider-column" 4 "+") ("narrower-column" -4 "-")))
  (destructuring-bind (name delta key) spec
    (register-command :component :scrolling :name name
                      :handler (lambda (snapshot event) (declare (ignore event))
                                 (resize-columns snapshot delta)))
    (bind-key :component :scrolling :key key :command name)))
(dolist (spec '(("pan-left" -16 "h") ("pan-right" 16 "l")))
  (destructuring-bind (name delta key) spec
    (register-command :component :scrolling :name name
                      :handler (lambda (snapshot event) (declare (ignore event))
                                 (pan snapshot delta)))
    (bind-key :component :scrolling :key key :command name)))
(dolist (spec '(("new-column" (:split :axis :columns) "c")
                ("close-column" (:close) "x") ("detach" (:detach) "d")
                ("reload" (:reload) "r")))
  (destructuring-bind (name operation key) spec
    (register-command :component :scrolling :name name
                      :handler (lambda (snapshot event) (declare (ignore snapshot event))
                                 (list (copy-tree operation))))
    (bind-key :component :scrolling :key key :command name)))
