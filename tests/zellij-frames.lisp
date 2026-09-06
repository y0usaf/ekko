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

(defun zellij-pane-test-pane (id outer &key layout activation visible name)
  (list :id id :outer-rect outer :layout-rect (or layout outer)
        :activation-order (or activation 0) :visible visible :name name))

(defun zellij-pane-test-snapshot (focus panes &optional viewport layout component-state)
  (list :focus focus :panes panes :viewport viewport
        :layout (or layout '(:columns 50 1 (:rows 50 2 3)))
        :component-state component-state))

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
                    '(:reported-cell-width 10 :reported-cell-height 40))))
    (zellij-frame-test-check
     (equal (zellij-pane-focus-action snapshot :right)
            '((:focus :pane 2)))
     "Fullscreen directional focus did not use :layout-rect")
    (zellij-frame-test-check
     (equal (zellij-pane-switch-focus-action snapshot)
            '((:focus :pane 2)))
     "Fullscreen cyclic focus did not include hidden sibling"))
  (let ((viewport '(:reported-cell-width 10 :reported-cell-height 40)))
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
              '(:reported-cell-width 2 :reported-cell-height 1)))
            '(1 :rows))
     "NoPreference half ratio did not round away from zero")
    (zellij-frame-test-check
     (null (zellij-pane-find-room-for-new-pane
            (zellij-pane-test-snapshot
             1 (list (zellij-pane-test-pane 1 '(0 0 20 20)))
             '(:reported-cell-width 100 :reported-cell-height 1))))
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
  (let* ((tree '(:columns 50 1 (:rows 50 2 3)))
         (snapshot (zellij-pane-test-snapshot
                    1 (list (zellij-pane-test-pane 1 '(0 0 10 10)
                                                     :activation 1)
                            (zellij-pane-test-pane 2 '(10 0 10 10)
                                                     :activation 3)
                            (zellij-pane-test-pane 3 '(10 0 10 10)
                                                     :activation 2))
                    nil tree)))
    (zellij-frame-test-check
     (equal (zellij-pane-move-direction-action snapshot :right)
            '((:set-layout :tree (:columns 50 2 (:rows 50 1 3)))))
     "Move right did not swap with the greatest activation target")
    (zellij-frame-test-check
     (equal (zellij-pane-move-cyclic-action snapshot nil)
            '((:set-layout :tree (:columns 50 2 (:rows 50 1 3)))))
     "Move next did not use row-major cyclic order")
    (zellij-frame-test-check
     (equal (zellij-pane-move-cyclic-action snapshot t)
            '((:set-layout :tree (:columns 50 3 (:rows 50 2 1)))))
     "Move previous did not wrap row-major order")
    (zellij-frame-test-check
     (null (zellij-pane-move-direction-action snapshot :up))
     "Move up produced an action without an adjacent target")
    (let ((zoomed (copy-tree snapshot)))
      (setf (getf zoomed :zoom) t)
      (zellij-frame-test-check
       (and (null (zellij-pane-move-cyclic-action zoomed nil))
            (null (zellij-pane-move-cyclic-action zoomed t))
            (null (zellij-pane-move-direction-action zoomed :right)))
       "Fullscreen tiled movement is ignored")))
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
  ;; Rename editing preserves the current label on entry, handles UTF-8
  ;; scalars and raw erase bytes, and stores a bounded per-pane undo value.
  (let* ((pane (zellij-pane-test-pane 1 '(0 0 20 10) :name "Aλ"))
         (snapshot (zellij-pane-test-snapshot
                    1 (list pane) nil nil
                    '(("zellij-modes" . ((99 "closed") (1 "stale"))))))
         (entry (zellij-pane-rename-enter-actions snapshot)))
    (zellij-frame-test-check
     (equal entry '((:set-state :value ((1 "Aλ"))) (:set-keymap :name :rename)))
     "Rename entry did not retain the old name and prune closed panes")
    (zellij-frame-test-check
     (equal (zellij-pane-rename-input-action snapshot '(:bytes (88)))
            '((:rename :pane 1 :text "AλX")))
     "Rename fallback did not append ASCII data")
    (zellij-frame-test-check
     (equal (zellij-pane-rename-input-action
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 1 '(0 0 20 10) :name "AλX")))
             '(:bytes (8)))
            '((:rename :pane 1 :text "Aλ")))
     "Rename backspace did not pop one Unicode scalar")
    (zellij-frame-test-check
     (equal (zellij-pane-rename-input-action
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 1 '(0 0 20 10) :name "AλX")))
             '(:bytes (127)))
            '((:rename :pane 1 :text "Aλ")))
     "Rename DEL did not pop one Unicode scalar")
    (zellij-frame-test-check
     (equal (zellij-pane-rename-input-action
             (zellij-pane-test-snapshot
              1 (list (zellij-pane-test-pane 1 '(0 0 20 10) :name "A")))
             '(:bytes (1 27 206 187)))
            '((:rename :pane 1 :text "Aλ")))
     "Rename control filtering did not retain decoded printable data")
    (zellij-frame-test-check
     (null (zellij-pane-rename-input-action
            (zellij-pane-test-snapshot
             1 (list (zellij-pane-test-pane 1 '(0 0 20 10) :name "A")))
            '(:bytes (194))))
     "Rename accepted incomplete UTF-8")
    (let ((undo-snapshot
            (zellij-pane-test-snapshot
             1 (list (zellij-pane-test-pane 1 '(0 0 20 10) :name "AλX")) nil nil
             '(("zellij-modes" (1 "Aλ"))))))
      (zellij-frame-test-check
       (equal (zellij-pane-rename-previous-action undo-snapshot)
              '((:rename :pane 1 :text "Aλ")
                (:set-keymap :name :pane)))
       "Rename Escape did not restore the captured name")))
  (format t "Zellij Pane helper tests passed~%")
  t)

(defun run-zellij-frame-tests ()
  (zellij-frame-test-check
   (string= (zellij-pane-title
             '(:name nil :terminal-title nil :launch-kind :command
               :argv ("/bin/echo" "a b" "--flag") :creation-position 4))
            "/bin/echo a b --flag")
   "Command title did not join argv literally")
  (zellij-frame-test-check
   (string= (zellij-pane-title
             '(:name nil :terminal-title nil :launch-kind :shell
               :argv nil :creation-position 7)) "Pane #7")
   "Implicit shell title did not use creation position")
  (zellij-frame-test-check
   (string= (zellij-pane-title
             '(:name "renamed" :terminal-title " osc " :launch-kind :command
               :argv ("cmd") :creation-position 4)) "renamed")
   "Rename did not outrank OSC title")
  (zellij-frame-test-check
   (string= (zellij-pane-title
             (list :name "" :terminal-title
                   (format nil "  ~C " (code-char #x2003))
                   :launch-kind :command :argv '("cmd") :creation-position 4)) "")
   "Whitespace-only OSC title did not remain explicit empty")
  (zellij-frame-test-check
   (string= (zellij-pane-title
             '(:name nil :terminal-title "" :launch-kind :command
               :argv ("cmd") :creation-position 4)) "")
   "Empty OSC title did not outrank command title")
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
      ;; Printable Unicode titles are supported. This check
      ;; guards against accidentally treating control text as decoration.
      (zellij-frame-test-check rejected "Control title unexpectedly accepted"))
    ;; Pinned unicode-width 0.1.10 uses scalar widths, including zero-width
    ;; marks retained by the frame ANSI path. These exact fit boundaries would
    ;; fail if the profile counted Lisp characters instead of display cells.
    (zellij-frame-test-check
     (equal (zellij-frame-title-left "界面" 7) '(" 界面 " 6))
     "Wide title fit mismatch")
    (zellij-frame-test-check
     (equal (zellij-frame-title-left "Café" 7) '(" Café " 6))
     "Combining title fit mismatch")
    (zellij-frame-test-check
     (equal (zellij-frame-title-left "界面界面界面" 10)
            '(" 界[..]面 " 10))
     "Wide title middle truncation mismatch")
    (zellij-frame-test-check
     (equal (zellij-frame-title-left "abcdef界" 9)
            '(" a[...] " 8))
     "Wide suffix budget must not admit half a scalar")
    (run-zellij-pane-tests)
    (format t "Zellij frame decoration tests passed~%")
    t))
