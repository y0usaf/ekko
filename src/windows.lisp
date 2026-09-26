(in-package #:ekko/runtime)

;; Geometry and gesture state belong to each view. Profiles declare which
;; spans are handles; applications retain their ordinary mouse protocol.
(defstruct window-drag owner pane kind x y from preview free target edge moved split tree)

(defun tiled-border-split (tree rectangles pane edge)
  "Find the actual visible split at PANE's edge, retaining its full-tree node.
Hidden leaves have no rectangles, so restoring them keeps their own ratios."
  (let ((rect (rest (assoc pane rectangles))))
    (labels ((bounds (node)
               (let ((rects (loop for id in (layout-pane-ids node)
                                  for r = (rest (assoc id rectangles)) when r collect r)))
                 (when rects
                   (list (reduce #'min rects :key #'first) (reduce #'min rects :key #'second)
                         (reduce #'max rects :key (lambda (r) (+ (first r) (third r))))
                         (reduce #'max rects :key (lambda (r) (+ (second r) (fourth r))))))))
             (walk (node)
               (unless (integerp node)
                 (or (walk (third node)) (walk (fourth node))
                     (let* ((columns (eq (first node) :columns))
                            (index (if columns 0 1))
                            (a (bounds (third node))) (b (bounds (fourth node)))
                            (before (member edge '(:right :bottom)))
                            (side (if before (third node) (fourth node))))
                       (when (and a b (member pane (layout-pane-ids side))
                                  (if columns (member edge '(:left :right)) (member edge '(:top :bottom)))
                                  (= (+ (nth index rect) (if before (nth (+ index 2) rect) 0))
                                     (if before (nth (+ index 2) a) (nth index b))))
                         (let ((cut (- (nth (+ index 2) a) (nth index a))))
                           (list node (+ cut (- (nth (+ index 2) b) (nth index b))) cut))))))))
      (when rect (walk tree)))))

(defun window-resize-split (view pane kind)
  (unless (or (pane-floating pane) (view-zoom view) (eq kind :move))
    (tiled-border-split (daemon-tree (view-daemon view))
                       (remove-if (lambda (r) (pane-floating (pane-by-id (view-daemon view) (first r))))
                                  (workspace-rectangles view))
                       (pane-id pane) kind)))
(defun window-intersects-p (view pane x y &optional (width 1))
  (when pane
    (destructuring-bind (ox oy w h) (shown-rect view (pane-id pane))
      (and (< ox (+ x width)) (< x (+ ox w)) (<= oy y) (< y (+ oy h))))))
(defun window-at (view x y)
  (find-if (lambda (p) (window-intersects-p view p x y)) (reverse (visible-panes view))))
(defun begin-window-drag (view owner span x y)
  (let ((pane (pane-by-id (view-daemon view) (getf span :pane))))
    (when pane
      (let* ((kind (getf span :drag)) (split (window-resize-split view pane kind)))
        (set-focus view (pane-id pane))
        (unless (or (eq kind :move) (pane-floating pane) split)
          (setf (view-chrome-press view) :dismiss)
          (return-from begin-window-drag))
        (setf (view-window-drag view)
              (make-window-drag :owner owner :pane pane :kind kind :x x :y y :split split
                                :tree (daemon-tree (view-daemon view))
                                :from (pane-outer-rect view pane) :preview (pane-outer-rect view pane)))))))

(defun tiled-drag-tree (drag dx dy)
  (destructuring-bind (node available cut) (window-drag-split drag)
    (let* ((delta (if (eq (first node) :columns) dx dy))
           (ratio (max 1 (min 99 (ceiling (* 100 (+ cut delta)) available)))))
      (if (zerop delta) (window-drag-tree drag)
          (subst (list (first node) ratio (third node) (fourth node)) node
                 (window-drag-tree drag) :test #'eq)))))

(defun tiled-drag-rectangles (view drag dx dy)
  (workspace-rectangles view nil (tiled-tree view (tiled-drag-tree drag dx dy))))
(defun drag-snap-target (view drag x y)
  (let ((target (find-if (lambda (p) (and (not (eq p (window-drag-pane drag)))
                                         (not (pane-floating p)) (window-intersects-p view p x y)))
                         (reverse (visible-panes view)))))
    (when target
      (destructuring-bind (ox oy w h) (pane-outer-rect view target)
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
                (otherwise (pane-outer-rect view target))))))))))
(defun viewport-snap-rect (view x y)
  "Content-area snap at the outermost cell: the top edge maximizes, the sides take a half."
  (multiple-value-bind (viewport pane gaps width height) (workspace-geometry view)
    (declare (ignore pane gaps))
    (let* ((left (fourth viewport)) (top (first viewport))
           (right (+ left width)) (half (max 12 (floor width 2))))
      (cond ((<= y top) (list left top width height))
            ((<= x left) (list left top half height))
            ((>= x (1- right)) (list (- right half) top half height))
            (t nil)))))
(defun floating-drag-rect (view drag dx dy)
  (destructuring-bind (ox oy w h) (window-drag-from drag)
    (let ((kind (window-drag-kind drag)))
      (if (eq kind :move)
          (clamp-window-rect view (list (+ ox dx) (+ oy dy) w h))
          (multiple-value-bind (viewport pane gaps width height) (workspace-geometry view)
            (declare (ignore pane gaps))
            (let* ((left (member kind '(:left :top-left :bottom-left)))
                   (right (member kind '(:right :top-right :bottom-right)))
                   (top (member kind '(:top :top-left :top-right)))
                   (bottom (member kind '(:bottom :bottom-left :bottom-right)))
                   (minw (min width 12)) (minh (min height 4))
                   (dx (cond (left (max (- (fourth viewport) ox) (min dx (- w minw))))
                             (right (max (- minw w) (min dx (- (+ (fourth viewport) width) ox w))))
                             (t 0)))
                   (dy (cond (top (max (- (first viewport) oy) (min dy (- h minh))))
                             (bottom (max (- minh h) (min dy (- (+ (first viewport) height) oy h))))
                             (t 0))))
              (list (if left (+ ox dx) ox) (if top (+ oy dy) oy)
                    (if left (- w dx) (+ w dx)) (if top (- h dy) (+ h dy)))))))))

(defun window-drag-mouse (view button x y up)
  (let ((drag (view-window-drag view)))
    (when drag
      ;; A removed source or a newer shared split invalidates this gesture.
      ;; Never apply a saved whole-tree resize over another view's edit.
      (unless (and (eq (window-drag-pane drag)
                       (pane-by-id (view-daemon view) (pane-id (window-drag-pane drag))))
                   (or (not (window-drag-split drag))
                       (equal (window-drag-tree drag) (daemon-tree (view-daemon view)))))
        (setf (view-window-drag view) nil)
        (incf (view-revision view))
        (return-from window-drag-mouse t))
      (let ((dx (- x (window-drag-x drag))) (dy (- y (window-drag-y drag)))
            (before (loop for (id . nil) in (drag-rects view) collect (cons id (shown-rect view id)))))
        (when (or (window-drag-moved drag)
                  (>= (+ (abs dx) (abs dy)) (if (eq (window-drag-kind drag) :move) 2 1)))
          (setf (window-drag-moved drag) t)
          (if (window-drag-split drag)
              (setf (window-drag-preview drag)
                    (tiled-drag-rectangles view drag dx dy))
              (progn
                (setf (window-drag-target drag) nil (window-drag-edge drag) nil
                      (window-drag-free drag) (floating-drag-rect view drag dx dy)
                      (window-drag-preview drag) (window-drag-free drag))
                (when (eq (window-drag-kind drag) :move)
                  (let ((snap (viewport-snap-rect view x y)))
                    (if snap
                        (setf (window-drag-target drag) nil (window-drag-edge drag) nil
                              (window-drag-preview drag) snap)
                        (multiple-value-bind (target edge preview) (drag-snap-target view drag x y)
                          (when target
                            (setf (window-drag-target drag) target (window-drag-edge drag) edge
                                  (window-drag-preview drag) preview))))))))
          (chase-drag view before)
          (incf (view-revision view)))
        (when up
          (release-drag view)
          (setf (view-window-drag view) nil)
          (when (and (not (window-drag-moved drag)) (= (logand button 3) 0)
                     (eq (window-drag-kind drag) :move))
            (let* ((id (pane-id (window-drag-pane drag))) (last (view-last-click view))
                   (again (and last (eql (first last) id) (< (- (now) (rest last)) 0.4))))
              (if again
                  (progn (setf (view-last-click view) nil)
                         (handler-case
                             (apply-actions view (window-drag-owner drag)
                                            (list (list :zoom :pane id)) nil nil)
                           (error (e) (note-error view e))))
                  (setf (view-last-click view) (cons id (now))))))
          (when (and (= (logand button 3) 0) (window-drag-moved drag))
            (handler-case
                (apply-actions view (window-drag-owner drag)
                  (list (cond ((window-drag-split drag)
                               (list :set-layout :tree (tiled-drag-tree drag dx dy)))
                              ((window-drag-target drag)
                               (let* ((target (window-drag-target drag)) (id (pane-id target)))
                                 (unless (eq target (pane-by-id (view-daemon view) id))
                                   (error "Window drag target no longer exists"))
                                 (list :place-window :pane (pane-id (window-drag-pane drag))
                                       :target id :edge (window-drag-edge drag))))
                              (t (list :float :pane (pane-id (window-drag-pane drag))
                                       :rect (window-drag-preview drag)))))
                  nil nil)
              (error (e) (note-error view e))))
          (incf (view-revision view))))
      t)))
(defun chase-drag (view before)
  "Held windows follow the pointer with a short glide from where they showed."
  (let ((seconds (animation-seconds view 1/3)))
    (when (plusp seconds)
      (loop for (id . rect) in (drag-rects view)
            for shown = (or (rest (assoc id before)) (committed-rect view id))
            unless (equal shown rect)
              do (move-window view id shown seconds (committed-rect view id))))))
(defun release-drag (view)
  "Hold released windows where they show, so a commit glides them into place
and a cancelled drag glides them home."
  (when (plusp (animation-seconds view))
    (loop for (id . nil) in (drag-rects view)
          do (move-window view id (shown-rect view id) (animation-seconds view 3/2)
                          (committed-rect view id) (pane-chrome view id)))))
(defun outline-spans (rect sgr)
  (destructuring-bind (x y w h) rect
    (append (list (list x y (make-string w :initial-element #\─) sgr))
            (when (> h 1) (list (list x (+ y h -1) (make-string w :initial-element #\─) sgr)))
            (loop for row from (1+ y) below (+ y h -1)
                  append (list (list x row "│" sgr) (list (+ x w -1) row "│" sgr))))))
(defun window-drag-overlays (view)
  "A drag moves the windows themselves; a snap outlines where the window will land."
  (let ((drag (view-window-drag view)))
    (when (and drag (window-drag-moved drag) (not (window-drag-split drag))
               (not (equal (window-drag-preview drag) (window-drag-free drag))))
      (let ((rect (window-drag-preview drag)))
        ;; The outline passes behind the held window.
        (graphics-safe-outlines view (outline-spans rect '(0 1 38 5 117)) (list (window-drag-pane drag)))))))

(defun update-window-hover (view button x y up)
  (let ((hover
          (when (and (= button 35) (not up) (not (view-popup view))
                     (not (view-window-drag view)) (not (view-drag view)))
            (multiple-value-bind (span owner) (decoration-at view x y)
              (when (and span owner
                         (or (getf span :hover-sgr)
                             (let ((pane (pane-by-id (view-daemon view) (getf span :pane))))
                               (and pane (getf span :drag) (not (eq (getf span :drag) :move))
                                    (or (pane-floating pane)
                                        (window-resize-split view pane (getf span :drag)))))))
                (list owner span))))))
    (unless (equal hover (view-window-hover view))
      (destructuring-bind (&optional owner span) (view-window-hover view)
        (when span (set-glow view (cons :hover (span-key owner span)) nil span owner)))
      (destructuring-bind (&optional owner span) hover
        (when span (set-glow view (cons :hover (span-key owner span)) t span owner)))
      (setf (view-window-hover view) hover)
      (incf (view-revision view)))))

(defun expand-span-rows (span)
  (loop for row from (getf span :y) below (+ (getf span :y) (getf span :rows 1))
        collect (list (getf span :x) row (getf span :text) (getf span :sgr))))
(defun window-hover-overlays (view)
  "Hover glows: each hovered span blends toward its :hover-sgr, and back after."
  (unless (view-popup view)
    (loop for glow in (view-glows view)
          for (key nil nil nil span owner) = glow
          for level = (glow-level view glow)
          for hover = (or (getf span :hover-sgr) '(0 1 38 5 117 48 5 235))
          when (and (eq (first key) :hover) (plusp level) (not (equal hover (getf span :sgr)))
                    (member span (rest (assoc owner (view-decorations view) :test #'equal)) :test #'equal))
            append (let* ((pane (pane-by-id (view-daemon view) (getf span :pane)))
                          (occluders (and pane (rest (member pane (visible-panes view))))))
                     (let ((fitted (window-span view owner span)))
                       (when fitted
                         (graphics-safe-outlines view
                           (loop for row from (getf fitted :y) below (+ (getf fitted :y) (getf fitted :rows 1))
                                 append (loop for piece in (span-pieces view owner fitted 0 nil)
                                              collect (list (getf piece :x) row (getf piece :text)
                                                            (blend-sgr (getf piece :sgr) hover level))))
                           occluders)))))))

(defun graphics-safe-outlines (view spans &optional occluders)
  ;; Transient strokes leave image placements intact; committed floating
  ;; geometry and menus still occlude images normally.
  (let ((graphics (remove-if-not
                   (lambda (p) (loop for image being the hash-values of (store-images (pane-graphics p))
                                     thereis (image-visible image))) (visible-panes view))))
    (loop for (x y text sgr alpha) in spans append
      (clip-decoration view graphics (list :x x :y y :text text :sgr sgr :alpha alpha) occluders))))
