(in-package #:ekko/runtime)

;; Geometry and gesture state belong to the daemon. Profiles declare which
;; spans are handles; applications retain their ordinary mouse protocol.
(defstruct window-drag owner pane kind x y from preview target edge moved)
(defun window-intersects-p (pane x y &optional (width 1))
  (and pane (< (pane-outer-x pane) (+ x width))
       (< x (+ (pane-outer-x pane) (pane-outer-cols pane)))
       (<= (pane-outer-y pane) y) (< y (+ (pane-outer-y pane) (pane-outer-rows pane)))))
(defun window-at (session x y)
  (find-if (lambda (p) (window-intersects-p p x y)) (reverse (visible-panes session))))
(defun begin-window-drag (session owner span x y)
  (let ((pane (pane-by-id session (getf span :pane))))
    (when pane
      (set-focus session (position pane (session-panes session)))
      (setf (session-window-drag session)
            (make-window-drag :owner owner :pane pane :kind (getf span :drag) :x x :y y
                              :from (pane-outer-rect pane) :preview (pane-outer-rect pane))))))
(defun drag-snap-target (session drag x y)
  (let ((target (find-if (lambda (p) (and (not (eq p (window-drag-pane drag)))
                                         (not (pane-floating p)) (window-intersects-p p x y)))
                         (reverse (visible-panes session)))))
    (when target
      (destructuring-bind (ox oy w h) (pane-outer-rect target)
        (let ((edge (cond ((< (- x ox) (max 2 (floor w 5))) :left)
                          ((< (- (+ ox w) x) (max 2 (floor w 5))) :right)
                          ((< (- y oy) (max 2 (floor h 5))) :top)
                          ((< (- (+ oy h) y) (max 2 (floor h 5))) :bottom)
                          ((not (pane-floating (window-drag-pane drag))) :center))))
          (when edge
            (values target edge
              (case edge
                (:left (list ox oy (max 1 (floor w 2)) h))
                (:right (list (+ ox (floor w 2)) oy (- w (floor w 2)) h))
                (:top (list ox oy w (max 1 (floor h 2))))
                (:bottom (list ox (+ oy (floor h 2)) w (- h (floor h 2))))
                (otherwise (pane-outer-rect target))))))))))
(defun window-drag-mouse (session button x y up)
  (let ((drag (session-window-drag session)))
    (when drag
      (let ((dx (- x (window-drag-x drag))) (dy (- y (window-drag-y drag))))
        (when (or (window-drag-moved drag) (>= (+ (abs dx) (abs dy)) 2))
          (setf (window-drag-moved drag) t)
          (destructuring-bind (ox oy w h) (window-drag-from drag)
            (let* ((kind (window-drag-kind drag))
                   (left (member kind '(:left :bottom-left)))
                   (right (member kind '(:right :bottom-right)))
                   (bottom (member kind '(:bottom :bottom-left :bottom-right)))
                   (rect (if (eq kind :move) (list (+ ox dx) (+ oy dy) w h)
                             (list (if left (+ ox (min dx (- w 12))) ox) oy
                                   (cond (left (- w (min dx (- w 12)))) (right (+ w dx)) (t w))
                                   (if bottom (+ h dy) h)))))
              (setf (window-drag-target drag) nil (window-drag-edge drag) nil
                    (window-drag-preview drag) (clamp-window-rect session rect))
              (when (eq kind :move)
                (multiple-value-bind (target edge preview) (drag-snap-target session drag x y)
                  (when target
                    (setf (window-drag-target drag) target (window-drag-edge drag) edge
                          (window-drag-preview drag) preview))))))
          (incf (session-revision session)))
        (when up
          (setf (session-window-drag session) nil)
          (when (and (= (logand button 3) 0) (window-drag-moved drag))
            (handler-case
                (apply-actions session (window-drag-owner drag)
                  (list (if (window-drag-target drag)
                            (list :place-window :pane (pane-id (window-drag-pane drag))
                                  :target (pane-id (window-drag-target drag)) :edge (window-drag-edge drag))
                            (list :float :pane (pane-id (window-drag-pane drag))
                                  :rect (window-drag-preview drag))))
                  nil nil)
              (error (e) (note-error session e))))
          (incf (session-revision session))))
      t)))
(defun outline-spans (rect sgr)
  (destructuring-bind (x y w h) rect
    (append (list (list x y (make-string w :initial-element #\─) sgr))
            (when (> h 1) (list (list x (+ y h -1) (make-string w :initial-element #\─) sgr)))
            (loop for row from (1+ y) below (+ y h -1)
                  append (list (list x row "│" sgr) (list (+ x w -1) row "│" sgr))))))
(defun window-drag-overlays (session)
  (let ((drag (session-window-drag session)))
    (when (and drag (window-drag-moved drag))
      (graphics-safe-outlines session (outline-spans (window-drag-preview drag)
                     (if (window-drag-target drag) '(0 1 38 5 117 48 5 235) '(0 38 5 250 48 5 235)))))))

(defun graphics-safe-outlines (session spans)
  ;; Transient strokes leave image placements intact; committed floating
  ;; geometry and menus still occlude images normally.
  (let ((graphics (remove-if-not
                   (lambda (p) (loop for image being the hash-values of (store-images (pane-graphics p))
                                     thereis (image-visible image))) (visible-panes session))))
    (loop for (x y text sgr) in spans append
      (clip-decoration session graphics (list :x x :y y :text text :sgr sgr)))))
