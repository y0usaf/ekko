(in-package #:cl-user)

;; The helper is an ASDF component in ekko/tests, so this only loads it for a
;; standalone invocation. Keep this fallback in Common Lisp so a bare pinned
;; SBCL can load the test without first loading ASDF/UIOP.
(eval-when (:load-toplevel :execute)
  (unless (fboundp 'zellij-frame-spans)
    (load (merge-pathnames
           "../examples/profiles/zellij-frames.lisp"
           (make-pathname :name nil :type nil :defaults *load-truename*)))))

(eval-when (:load-toplevel :execute)
  (unless (fboundp 'zellij-pane-find-room-for-new-pane)
    (load (merge-pathnames
           "../examples/profiles/zellij-pane.lisp"
           (make-pathname :name nil :type nil :defaults *load-truename*)))))

(defun zellij-frame-test-check (condition format-control &rest arguments)
  (unless condition
    (apply #'error format-control arguments)))

(defun zellij-frame-test-top (spans)
  (getf (first spans) :text))

(defun zellij-frame-test-sgr (spans)
  (getf (first spans) :sgr))

(defun zellij-pane-test-pane (id outer &key layout activation visible)
  (list :id id :outer-rect outer :layout-rect (or layout outer)
        :activation-order (or activation 0) :visible visible))

(defun zellij-pane-test-snapshot (focus panes &optional viewport)
  (list :focus focus :panes panes :viewport viewport))

(defun run-zellij-pane-tests ()
  ;; Directional and cyclic focus use the tiled rectangles even while the
  ;; focused pane owns the fullscreen outer rectangle.
  (let* ((left (zellij-pane-test-pane 1 '(0 0 40 20)
                                      :layout '(0 0 20 20) :activation 3 :visible t))
         (hidden-right (zellij-pane-test-pane 2 '(0 0 40 20)
                                               :layout '(20 0 20 20)
                                               :activation 1 :visible nil))
         (snapshot (zellij-pane-test-snapshot
                    1 (list left hidden-right)
                    '(:cell-width 10 :cell-height 40))))
    (zellij-frame-test-check
     (equal (zellij-pane-focus-action snapshot :right)
            '((:focus :pane 2)))
     "Fullscreen directional focus did not use :layout-rect")
    (zellij-frame-test-check
     (equal (zellij-pane-switch-focus-action snapshot)
            '((:focus :pane 2)))
     "Fullscreen cyclic focus did not include hidden sibling"))
  (let ((viewport '(:cell-width 10 :cell-height 40)))
    ;; The terminal ratio is four.  The 5x10 boundary is ineligible because
    ;; neither dimension exceeds ten, while 5x11 selects rows.
    (zellij-frame-test-check
     (null (zellij-pane-find-room-for-new-pane
            (zellij-pane-test-snapshot
             1 (list (zellij-pane-test-pane 1 '(0 0 5 10))) viewport)))
     "NoPreference accepted a 5x10 pane")
    (zellij-frame-test-check
     (equal (zellij-pane-find-room-for-new-pane
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 1 '(0 0 5 11))) viewport))
            '(1 :rows))
     "NoPreference 5x11 boundary did not choose rows")
    (zellij-frame-test-check
     (equal (zellij-pane-find-room-for-new-pane
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 1 '(0 0 11 5))) viewport))
            '(1 :columns))
     "NoPreference 11x5 boundary did not choose columns")
    ;; Equal scores retain the lower ID despite reversed input order.  A
    ;; zero rounded ratio follows the upstream initial score of zero and
    ;; produces no candidate.
    (zellij-frame-test-check
     (equal (zellij-pane-find-room-for-new-pane
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 2 '(0 0 11 5))
                      (zellij-pane-test-pane 1 '(0 0 11 5))) viewport))
            '(1 :columns))
     "NoPreference tie did not retain lower pane ID")
    ;; A half ratio rounds away from zero, matching Rust f64::round.  With a
    ;; ratio of one, 10x11 chooses rows; ties-to-even would incorrectly yield
    ;; zero and reject both axes at this boundary.
    (zellij-frame-test-check
     (equal (zellij-pane-find-room-for-new-pane
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 1 '(0 0 10 11)))
              '(:cell-width 2 :cell-height 1)))
            '(1 :rows))
     "NoPreference half ratio did not round away from zero")
    (zellij-frame-test-check
     (null (zellij-pane-find-room-for-new-pane
            (zellij-pane-test-snapshot
             1 (list (zellij-pane-test-pane 1 '(0 0 20 20)))
             '(:cell-width 100 :cell-height 1))))
     "NoPreference accepted a zero-ratio pane")
    (zellij-frame-test-check
     (zellij-pane-room-p (zellij-pane-test-pane 1 '(0 0 10 8)) :columns)
     "Column split rejected the width-10 boundary")
    (zellij-frame-test-check
     (not (zellij-pane-room-p (zellij-pane-test-pane 1 '(0 0 10 8)) :rows))
     "Row split accepted the height-8 boundary"))
  (let ((snapshot
          (append
           (zellij-pane-test-snapshot
            1 (list (zellij-pane-test-pane 1 '(0 0 10 8))))
           '(:zoom t))))
    (zellij-frame-test-check
     (equal (zellij-pane-split-actions :rows 1 snapshot)
            '((:zoom)
              (:set-keymap :name :normal)
              (:pane-note :pane 1 :text "CAN'T SPLIT!"
               :sgr (0 1 38 5 124 49) :duration 1000)))
     "Failed split did not restore zoom, mode, and pane note"))
  (let ((snapshot
          (append
           (zellij-pane-test-snapshot
            1 (list (zellij-pane-test-pane 1 '(0 0 5 10))))
           '(:zoom t))))
    (zellij-frame-test-check
     (equal (zellij-pane-no-preference-actions snapshot)
            '((:zoom) (:set-keymap :name :normal)))
     "NoPreference no-room branch was not silent or did not clear zoom"))
  ;; Close names the most recently active surviving pane, as required by the
  ;; public :close :focus contract.
  (let ((snapshot
          (zellij-pane-test-snapshot
           1 (list (zellij-pane-test-pane 1 '(0 0 10 10) :activation 9)
                   (zellij-pane-test-pane 2 '(10 0 10 10) :activation 4)
                   (zellij-pane-test-pane 3 '(20 0 10 10) :activation 7)))))
    (zellij-frame-test-check
     (equal (zellij-pane-close-actions snapshot)
            '((:close :focus 3) (:set-keymap :name :normal)))
     "Close did not select the most recently active survivor"))
  (format t "Zellij Pane helper tests passed~%")
  t)

(defun run-zellij-frame-tests ()
  (let ((short-a (zellij-frame-spans
                  '(:rect (0 0 40 24) :title "A" :scroll (0 1)
                    :focus t :mode :normal)))
        (short-b (zellij-frame-spans
                  '(:rect (40 0 40 24) :title "B" :scroll (0 1)
                    :focus nil :mode :normal)))
        (small (zellij-frame-spans
                '(:rect (0 0 10 8) :title "LONG-PANE-A-0123456789"
                  :scroll (0 1) :focus t :mode :normal)))
        (small-fullscreen (zellij-frame-spans
                           '(:rect (0 0 20 8) :title "LONG-PANE-A-0123456789"
                             :scroll (0 1) :focus t :mode :normal)))
        (pane (zellij-frame-spans
               '(:rect (0 0 40 24) :title "A" :scroll (0 1)
                 :focus t :mode :pane)))
        (note (zellij-frame-spans
               '(:rect (0 0 40 24) :title "CAN'T SPLIT!" :scroll (0 0)
                 :focus t :mode :normal :sgr (0 1 38 5 124 49)))))
    (zellij-frame-test-check
     (string= (zellij-frame-test-top short-a)
              "┌ A ───────────────────── SCROLL:  0/1 ┐")
     "80x24 focused title mismatch: ~S" (zellij-frame-test-top short-a))
    (zellij-frame-test-check
     (string= (zellij-frame-test-top short-b)
              "┌ B ───────────────────── SCROLL:  0/1 ┐")
     "80x24 unfocused title mismatch: ~S" (zellij-frame-test-top short-b))
    (zellij-frame-test-check
     (equal (zellij-frame-test-sgr short-a) '(0 1 38 5 154 49))
     "Normal focus SGR mismatch: ~S" (zellij-frame-test-sgr short-a))
    (zellij-frame-test-check
     (equal (zellij-frame-test-sgr short-b) '(0 1 39 49))
     "Unfocused SGR mismatch: ~S" (zellij-frame-test-sgr short-b))
    (zellij-frame-test-check
     (equal (zellij-frame-test-sgr pane) '(0 1 38 5 166 49))
     "Pane focus SGR mismatch: ~S" (zellij-frame-test-sgr pane))
    (zellij-frame-test-check
     (equal (zellij-frame-test-sgr note) '(0 1 38 5 124 49))
     "Pane note SGR override mismatch: ~S" (zellij-frame-test-sgr note))
    (zellij-frame-test-check
     (string= (zellij-frame-test-top small) "┌ L[..]9 ┐")
     "20x8 title truncation mismatch: ~S" (zellij-frame-test-top small))
    (zellij-frame-test-check
     (string= (zellij-frame-test-top small-fullscreen)
              "┌ LONG-P[..]456789 ┐")
     "20x8 fullscreen title mismatch: ~S"
     (zellij-frame-test-top small-fullscreen))
    (zellij-frame-test-check (and (= (length short-a) 4)
                                  (= (loop for span in short-a sum (getf span :rows 1)) 46))
                             "Expected four compact spans expanding to 46 rows")
    (zellij-frame-test-check
     (equal (list (getf (first short-a) :x) (getf (first short-a) :y)
                  (getf (first (last short-a)) :x)
                  (getf (first (last short-a)) :y))
            '(0 0 0 23))
     "Frame coordinates mismatch")
    (let ((rejected nil))
      (handler-case
          (zellij-frame-spans
           (list :rect '(0 0 10 8) :title (format nil "bad~C" #\Newline)
                 :scroll '(0 0) :focus t :mode :normal))
        (error () (setf rejected t)))
      ;; Printable ASCII titles are the explicit helper contract. This check
      ;; guards against accidentally treating control text as decoration.
      (zellij-frame-test-check rejected "Control title unexpectedly accepted"))
    (run-zellij-pane-tests)
    (format t "Zellij frame decoration tests passed~%")
    t))
