(defpackage #:ekko/layout
  (:use #:cl)
  (:export #:split-pane #:remove-pane #:resize-split #:swap-panes #:rectangles #:minimum-size))
(in-package #:ekko/layout)

;; Leaves are stable pane IDs. Branches are (axis percentage first second).
;; A leaf includes one header row and at least one application row.
(defun minimum-size (tree)
  (if (integerp tree) '(1 2)
      (destructuring-bind (axis ratio a b) tree
        (declare (ignore ratio))
        (destructuring-bind (aw ah) (minimum-size a)
          (destructuring-bind (bw bh) (minimum-size b)
            (if (eq axis :columns) (list (+ aw bw 1) (max ah bh))
                (list (max aw bw) (+ ah bh))))))))
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
(defun rectangles (tree cols rows focus &optional zoom)
  "Return (id x y width height), including headers. Hide siblings if too small."
  (destructuring-bind (mw mh) (minimum-size tree)
    (when (or zoom (< cols mw) (< rows mh)) (setf tree focus)))
  (labels ((walk (node x y w h)
             (if (integerp node) (list (list node x y w h))
                 (destructuring-bind (axis ratio a b) node
                   (let* ((columns (eq axis :columns)) (index (if columns 0 1))
                          (available (- (if columns w h) (if columns 1 0)))
                          (cut (max (nth index (minimum-size a))
                                    (min (- available (nth index (minimum-size b)))
                                         (floor (* available ratio) 100)))))
                     (if columns
                         (append (walk a x y cut h) (walk b (+ x cut 1) y (- available cut) h))
                         (append (walk a x y w cut) (walk b x (+ y cut) w (- available cut)))))))))
    (walk tree 0 0 cols rows)))
