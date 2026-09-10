;;;; Pure regression/property tests for graceful layout overflow in ekko/layout.
;;;; Load with: sbcl --load src/layout.lisp --load tests/layout_capacity.lisp
(defpackage #:ekko/layout-capacity
  (:use #:cl))
(in-package #:ekko/layout-capacity)

(defparameter *pass* 0)
(defparameter *fail* nil)

(defmacro expect (desc form)
  `(let ((ok ,form))
     (if ok (incf *pass*)
         (push ,desc *fail*))))

(defmacro expect-equal (desc got want)
  `(let ((g ,got) (w ,want))
     (if (equal g w) (incf *pass*)
         (push (format nil "~A: got ~S want ~S" ,desc g w) *fail*))))

(defun rect (rects id)
  (find id rects :key #'first))

(defun ids-present-p (rects ids)
  (and (= (length rects) (length ids))
       (loop for id in ids always (rect rects id))))

(defun in-bounds-p (rect cols rows)
  (destructuring-bind (id x y w h) rect
    (declare (ignore id))
    (and (>= x 0) (>= y 0) (>= w 1) (>= h 1)
         (<= (+ x w) cols) (<= (+ y h) rows))))

(defun non-overlap-p (rects)
  (loop for (a . rest) on rects
        do (dolist (b rest)
             (destructuring-bind (_ ax ay aw ah) a
               (declare (ignore _))
               (destructuring-bind (_2 bx by bw bh) b
                 (declare (ignore _2))
                 (when (and (< ax (+ bx bw)) (< bx (+ ax aw))
                            (< ay (+ by bh)) (< by (+ ay ah)))
                   (return-from non-overlap-p nil))))))
  t)

(defun layout (tree cols rows focus &rest keys)
  "Call rectangles with zoom NIL; FOCUS is a leaf id."
  (apply #'ekko/layout:rectangles tree cols rows focus nil keys))

(defun leaves (tree)
  (if (integerp tree) (list tree)
      (append (leaves (third tree)) (leaves (fourth tree)))))

(defun min-of (leaf leaf-min)
  (if (functionp leaf-min) (funcall leaf-min leaf) leaf-min))

;;; Contract check for one viewport: every visible rectangle is in bounds and
;;; non-overlapping; each visible leaf gets at least its minimum in each
;;; dimension, clipped only where the viewport itself is smaller than the
;;; minimum in that dimension ((>= w (min mw cols)) etc.); FOCUS is always
;;; visible, clipped to the viewport when even its minimum does not fit; the
;;; tree is not mutated.
(defun valid-overflow-p (tree cols rows focus leaf-min
                         &key (column-gap 1) (row-gap 0))
  (let* ((snapshot (copy-tree tree))
         (r (layout tree cols rows focus
                    :leaf-min leaf-min :column-gap column-gap :row-gap row-gap)))
    (and (equal tree snapshot)
         (rect r focus)
         (non-overlap-p r)
         (every (lambda (x) (in-bounds-p x cols rows)) r)
         (loop for (id x y w h) in r
               for (mw mh) = (min-of id leaf-min)
               always (and (>= w (min mw cols)) (>= h (min mh rows))))
         ;; Focus meets its minimum when the viewport allows it, else it is
         ;; clipped to (no larger than) the viewport.
         (if (and (>= cols (first (min-of focus leaf-min)))
                  (>= rows (second (min-of focus leaf-min))))
             (and (>= (fourth (rect r focus)) (first (min-of focus leaf-min)))
                  (>= (fifth (rect r focus)) (second (min-of focus leaf-min))))
             (and (<= (fourth (rect r focus)) cols)
                  (<= (fifth (rect r focus)) rows))))))

(defun run-tests ()
  ;; --- Acceptance regression: asymmetric overflow keeps fitting siblings ---
  ;; Parent-reported failure: focus 1 (3 wide) plus either pane 2 or 3 (3 wide)
  ;; fit as two 3-wide panes + gap at width 7, yet the tree collapsed to one
  ;; full-viewport pane because the FULL subtree minimum (9) did not fit.
  (expect "asymmetric tree at width 7 keeps focus + one 3-wide sibling"
          (let* ((r (ekko/layout:rectangles
                     '(:columns 50 1 (:columns 50 2 3)) 7 5 1 nil :leaf-min '(3 3))))
            (and (= (length r) 2)
                 (rect r 1) (rect r 2) (null (rect r 3))
                 (every (lambda (x) (in-bounds-p x 7 5)) r)
                 (non-overlap-p r)
                 (loop for (id x y w h) in r
                       always (and (= h 5) (>= w 3))))))
  (expect "asymmetric overflow: surviving sibling holds its full minimum"
          (let* ((r (ekko/layout:rectangles
                     '(:columns 50 1 (:columns 50 2 3)) 7 5 1 nil :leaf-min '(3 3)))
                 (r2 (rect r 2)))
            (and r2 (= (fourth r2) 3))))
  (expect "asymmetric tree fully fits at width 11 unchanged"
          (let* ((r (ekko/layout:rectangles
                     '(:columns 50 1 (:columns 50 2 3)) 11 5 1 nil :leaf-min '(3 3))))
            (ids-present-p r '(1 2 3))))
  ;; --- Baseline collapses retained -----------------------------------------
  (expect "width1: collapses to focus only, in bounds"
          (let ((r (ekko/layout:rectangles '(:columns 50 1 2) 1 5 1 nil :leaf-min '(3 3))))
            (and (= (length r) 1) (eql (first (rect r 1)) 1)
                 (in-bounds-p (rect r 1) 1 5))))
  (expect "width6: two 3-wide panes + gap cannot fit; one pane keeps minimum"
          (let* ((r (ekko/layout:rectangles '(:columns 50 1 2) 6 5 1 nil :leaf-min '(3 3)))
                 (r1 (rect r 1)) (r2 (rect r 2)))
            (and (= (length r) 1) r1 (null r2)
                 (>= (fourth r1) 3) (>= (fifth r1) 3)
                 (in-bounds-p r1 6 5))))
  (expect "sub-minimum viewport: only focus survives"
          (let* ((r (ekko/layout:rectangles '(:columns 50 1 2) 2 2 1 nil :leaf-min '(3 3))))
            (and (= (length r) 1) (eql (first (rect r 1)) 1)
                 (in-bounds-p (rect r 1) 2 2))))
  ;; --- Property: geometry valid across the FULL small-viewport grid --------
  (expect "full grid: single columns split never violates contract"
          (loop for cols from 1 to 8
                always (loop for rows from 1 to 8
                             always (valid-overflow-p '(:columns 50 1 2) cols rows 1 '(3 3)))))
  (expect "full grid: rows split with leaf-min 2x3"
          (loop for cols from 1 to 8
                always (loop for rows from 1 to 8
                             always (valid-overflow-p '(:rows 50 1 2) cols rows 2 '(2 3)))))
  (expect "full grid: mixed 2x2 tree, desktop minimum 3x3"
          (loop for cols from 1 to 9
                always (loop for rows from 1 to 9
                             always (valid-overflow-p
                                     '(:columns 50 (:rows 50 1 2) (:rows 50 3 4))
                                     cols rows 1 '(3 3)))))
  (expect "full grid: all focus leaves on mixed tree, min 2x2"
          (loop for focus in '(1 2 3 4)
                always (loop for cols from 1 to 8
                             always (loop for rows from 1 to 8
                                          always (valid-overflow-p
                                                  '(:columns 50 (:rows 50 1 2) (:rows 50 3 4))
                                                  cols rows focus '(2 2))))))
  (expect "full grid: asymmetric tree, desktop minimum 3x3"
          (loop for cols from 1 to 10
                always (loop for rows from 1 to 6
                             always (valid-overflow-p
                                     '(:columns 50 1 (:columns 50 2 3))
                                     cols rows 1 '(3 3)))))
  (expect "gap >= viewport renders focus clipped, nothing out of bounds"
          (loop for cols from 1 to 4
                always (loop for rows from 1 to 4
                             always (valid-overflow-p '(:columns 50 1 2) cols rows 1 '(3 3)
                                                      :column-gap 5 :row-gap 5))))
  (expect "full grid: function leaf-min per pane"
          (loop for cols from 1 to 8
                always (loop for rows from 1 to 8
                             always (valid-overflow-p
                                     '(:columns 50 (:rows 50 1 2) (:rows 50 3 4))
                                     cols rows 1
                                     (lambda (id) (if (evenp id) '(1 1) '(2 3)))))))
  (expect "deep tree (8 leaves) stays in bounds at every small viewport"
          (let ((tree 1))
            (loop for i from 2 to 8
                  do (setf tree (ekko/layout:split-pane tree (1- i) i :columns)))
            (loop for cols from 1 to 10
                  always (loop for rows from 1 to 6
                               always (valid-overflow-p tree cols rows 1 '(2 2))))))
  ;; --- Capacity: fitting panes are retained --------------------------------
  (expect "width7 fits both panes at their minimum"
          (let* ((r (ekko/layout:rectangles '(:columns 50 1 2) 7 5 1 nil :leaf-min '(3 3))))
            (and (= (length r) 2)
                 (= (fourth (rect r 1)) 3) (= (fourth (rect r 2)) 3)
                 (non-overlap-p r)
                 (every (lambda (x) (in-bounds-p x 7 5)) r))))
  (expect "mixed tree: viewport sized for all four renders all at min 2x2"
          (let* ((r (layout '(:columns 50 (:rows 50 1 2) (:rows 50 3 4)) 5 4 1
                            :leaf-min '(2 2))))
            (and (ids-present-p r '(1 2 3 4))
                 (non-overlap-p r)
                 (every (lambda (x) (in-bounds-p x 5 4)) r)
                 (loop for (id x y w h) in r
                       always (and (>= w 2) (>= h 2))))))
  (expect "mixed tree 4x4: two stacked rows fit with no row gap"
          (let* ((r (layout '(:columns 50 (:rows 50 1 2) (:rows 50 3 4)) 4 4 1
                            :leaf-min '(2 2))))
            (and (= (length r) 2)
                 (rect r 1) (rect r 2)
                 (non-overlap-p r)
                 (loop for (id x y w h) in r always (and (= w 4) (>= h 2))))))
  (expect "mixed tree 3x3: focus fills viewport"
          (let* ((r (layout '(:columns 50 (:rows 50 1 2) (:rows 50 3 4)) 3 3 1
                            :leaf-min '(2 2))))
            (and (= (length r) 1) (= (fourth (rect r 1)) 3) (= (fifth (rect r 1)) 3))))
  (expect "full fitting at desktop min 3x3 renders all four panes"
          (let* ((r (layout '(:columns 50 (:rows 50 1 2) (:rows 50 3 4)) 7 7 1
                            :leaf-min '(3 3))))
            (and (ids-present-p r '(1 2 3 4))
                 (non-overlap-p r)
                 (every (lambda (x) (in-bounds-p x 7 7)) r)
                 (loop for (id x y w h) in r
                       always (and (>= w 3) (>= h 3))))))
  ;; --- Focus any leaf: always visible and in bounds ------------------------
  (expect "focusing each leaf keeps focus visible across tiny viewports"
          (loop for focus in '(1 2 3 4)
                always (loop for cols from 1 to 4
                             always (loop for rows from 1 to 4
                                          always (let ((r (layout '(:columns 50 (:rows 50 1 2)
                                                                            (:rows 50 3 4))
                                                                  cols rows focus)))
                                                   (and (rect r focus)
                                                        (non-overlap-p r)
                                                        (every (lambda (x) (in-bounds-p x cols rows)) r)))))))
  ;; --- Explicit zoom: focus-only, others dropped ---------------------------
  (expect "explicit zoom is focus-only"
          (let* ((r (ekko/layout:rectangles
                     '(:columns 50 (:rows 50 1 2) (:rows 50 3 4))
                     40 10 1 t)))
            (and (= (length r) 1) (eql (first (rect r 1)) 1)
                 (= (fourth (rect r 1)) 40) (= (fifth (rect r 1)) 10))))
  (expect "zoom only collapses when explicitly requested, not on overflow"
          (let* ((r (ekko/layout:rectangles
                     '(:columns 50 (:rows 50 1 2) (:rows 50 3 4))
                     3 3 1 t)))
            (= (length r) 1)))
  ;; --- Tree is not mutated --------------------------------------------------
  (expect "original tree not mutated by rectangles"
          (let* ((tree (list :columns 50 (list :rows 50 1 2) (list :rows 50 3 4)))
                 (snapshot (copy-tree tree)))
            (layout tree 3 3 2)
            (equal tree snapshot)))
  (expect "overflow reduction does not mutate the original tree"
          (let* ((tree '(:columns 50 1 (:columns 50 2 3)))
                 (snapshot (copy-tree tree)))
            (ekko/layout:rectangles tree 7 5 1 nil :leaf-min '(3 3))
            (equal tree snapshot)))
  ;; --- Resize up restores ----------------------------------------------------
  (expect "growing viewport restores full layout without gaps"
          (let* ((tree '(:columns 50 (:rows 50 1 2) (:rows 50 3 4)))
                 (small (layout tree 3 3 1))
                 (big (layout tree 40 10 1)))
            (and (ids-present-p big '(1 2 3 4))
                 (every (lambda (x) (in-bounds-p x 40 10)) big)
                 (non-overlap-p big)
                 (>= (length big) (length small))
                 (< (fourth (rect small 1)) (fourth (rect big 1))))))
  ;; --- Fitting layouts unchanged ---------------------------------------------
  (expect "fitting layout with room to spare keeps ratios"
          (let* ((r (layout '(:columns 50 1 2) 41 10 1))
                 (w1 (fourth (rect r 1))) (w2 (fourth (rect r 2))))
            (and (= (+ w1 w2 1) 41)
                 (>= w1 2) (>= w2 2)
                 (<= (abs (- w1 w2)) 1))))
  ;; --- Leaf minimum function ---------------------------------------------------
  (expect "leaf-min function honored per pane when space allows"
          (let* ((r (ekko/layout:rectangles '(:columns 50 1 2) 30 10 1
                                            nil :leaf-min (lambda (id) (if (= id 1) '(4 2) '(1 2)))))
                 (w1 (fourth (rect r 1))))
            (>= w1 4)))
  (expect "varied per-pane minima respected across the full grid"
          (loop for cols from 1 to 9
                always (loop for rows from 1 to 9
                             always (valid-overflow-p
                                     '(:columns 50 1 2) cols rows 1
                                     (lambda (id) (if (= id 1) '(4 3) '(2 2)))))))
  ;; --- Gaps configurable ---------------------------------------------------------
  (expect "column gap configurable"
          (let* ((r (ekko/layout:rectangles '(:columns 50 1 2) 41 10 1
                                            nil :column-gap 3))
                 (w1 (fourth (rect r 1))) (w2 (fourth (rect r 2))))
            (= (+ w1 w2 3) 41)))
  (expect "row gap configurable"
          (let* ((r (ekko/layout:rectangles '(:rows 50 1 2) 10 41 1
                                            nil :row-gap 2))
                 (h1 (fifth (rect r 1))) (h2 (fifth (rect r 2))))
            (= (+ h1 h2 2) 41)))
  ;; --- 16 panes runtime max --------------------------------------------------------
  (let ((tree16 1))
    (loop for i from 2 to 16
          do (setf tree16 (ekko/layout:split-pane tree16 1 i :columns)))
    (expect "16 panes all fit at width 40"
            (let* ((r (layout tree16 40 8 1)))
              (and (ids-present-p r (loop for i from 1 to 16 collect i))
                   (non-overlap-p r)
                   (every (lambda (x) (in-bounds-p x 40 8)) r))))
    ;; 16 leaves need 16+15=31 columns; 15 need 29, so width 30 keeps 15.
    (expect "16 panes at width 30 drop exactly the newest pane"
            (let* ((r (layout tree16 30 8 1)))
              (and (ids-present-p r (loop for i from 1 to 15 collect i))
                   (null (rect r 16))
                   (non-overlap-p r)
                   (every (lambda (x) (in-bounds-p x 30 8)) r))))
    (expect "16 panes collapse to focus on a tiny viewport, in bounds"
            (let* ((r (layout tree16 4 4 1)))
              (and (every (lambda (x) (in-bounds-p x 4 4)) r)
                   (rect r 1)
                   (non-overlap-p r))))
    (expect "16 panes: full grid contract holds on a mid viewport"
            (loop for cols from 26 to 34
                  always (loop for rows from 1 to 8
                               always (valid-overflow-p tree16 cols rows 1 '(1 2))))))
  (values *pass* (reverse *fail*)))

(run-tests)
(if *fail*
    (progn
      (format t "FAILURES (~D):~%" (length *fail*))
      (dolist (f (reverse *fail*)) (format t "  - ~A~%" f))
      (sb-ext:quit :unix-status 1))
    (progn
      (format t "ALL PASS: ~D checks~%" *pass*)
      (sb-ext:quit :unix-status 0)))
