(defpackage #:ekko/layout
  (:use #:cl)
  (:export #:split-pane #:remove-pane #:resize-split #:swap-panes #:rectangles #:minimum-size))
(in-package #:ekko/layout)

;; Leaves are stable pane IDs. Branches are (axis percentage first second).
;; A leaf's minimum is supplied by the caller so layout can reserve the
;; pane's requested frame and still keep one cell for application content.
(defun minimum-size (tree &key (leaf-min '(1 2)) (column-gap 1) (row-gap 0))
  (if (integerp tree) (copy-list (if (functionp leaf-min) (funcall leaf-min tree) leaf-min))
      (destructuring-bind (axis ratio a b) tree
        (declare (ignore ratio))
        (destructuring-bind (aw ah) (minimum-size a :leaf-min leaf-min :column-gap column-gap :row-gap row-gap)
          (destructuring-bind (bw bh) (minimum-size b :leaf-min leaf-min :column-gap column-gap :row-gap row-gap)
            (if (eq axis :columns) (list (+ aw bw column-gap) (max ah bh))
                (list (max aw bw) (+ ah bh row-gap))))))))
(defun split-pane (tree target new axis)
  (if (integerp tree)
      (if (= tree target) (list axis 50 tree new) tree)
      (list (first tree) (second tree)
            (split-pane (third tree) target new axis)
            (split-pane (fourth tree) target new axis))))
(defun remove-pane (tree target)
  (if (integerp tree) (unless (= tree target) tree)
      (let ((a (remove-pane (third tree) target)) (b (remove-pane (fourth tree) target)))
        (cond ((null a) b) ((null b) a) (t (list (first tree) (second tree) a b))))))
(defun resize-split (tree target delta)
  "Resize the nearest split containing TARGET; return tree and found flag."
  (if (integerp tree) (values tree (= tree target))
      (destructuring-bind (axis ratio a b) tree
        (cond ((or (eql a target) (eql b target))
               (values (list axis (max 10 (min 90 (+ ratio delta))) a b) t))
              (t (multiple-value-bind (new found) (resize-split a target delta)
                   (if found (values (list axis ratio new b) t)
                       (multiple-value-bind (new found) (resize-split b target delta)
                         (values (list axis ratio a new) found)))))))))
(defun swap-panes (tree a b)
  (if (integerp tree) (cond ((= tree a) b) ((= tree b) a) (t tree))
      (list (first tree) (second tree) (swap-panes (third tree) a b) (swap-panes (fourth tree) a b))))
(defun rectangles (tree cols rows focus &optional zoom
                  &key (leaf-min '(1 2)) (column-gap 1) (row-gap 0))
  "Return (id x y width height) outer rectangles, collapsing when too small."
  (destructuring-bind (mw mh) (minimum-size tree :leaf-min leaf-min :column-gap column-gap :row-gap row-gap)
    (when (or zoom (< cols mw) (< rows mh)) (setf tree focus)))
  ;; Each branch previously re-ran minimum-size over its whole subtree,
  ;; making layout quadratic in depth. Cache branch minima (eq-keyed,
  ;; scoped to this call) so each is computed once; integer leaves still
  ;; consult leaf-min exactly as often as before.
  (let ((mins (make-hash-table :test #'eq)))
    (labels ((min-of (node)
               (if (integerp node)
                   (copy-list (if (functionp leaf-min) (funcall leaf-min node) leaf-min))
                   (multiple-value-bind (v foundp) (gethash node mins)
                     (if foundp v
                         (destructuring-bind (axis ratio a b) node
                           (declare (ignore ratio))
                           (destructuring-bind (aw ah) (min-of a)
                             (destructuring-bind (bw bh) (min-of b)
                               (setf (gethash node mins)
                                     (if (eq axis :columns)
                                         (list (+ aw bw column-gap) (max ah bh))
                                         (list (max aw bw) (+ ah bh row-gap)))))))))))
             (walk (node x y w h)
               (if (integerp node) (list (list node x y w h))
                   (destructuring-bind (axis ratio a b) node
                     (let* ((columns (eq axis :columns)) (index (if columns 0 1))
                            (gap (if columns column-gap row-gap))
                            (available (- (if columns w h) gap))
                            (amin (nth index (min-of a)))
                            (bmin (nth index (min-of b)))
                            (cut (max amin
                                      (min (- available bmin)
                                           (floor (* available ratio) 100)))))
                       (if columns
                           (append (walk a x y cut h) (walk b (+ x cut gap) y (- available cut) h))
                           (append (walk a x y w cut) (walk b x (+ y cut gap) w (- available cut)))))))))
      (walk tree 0 0 cols rows))))
