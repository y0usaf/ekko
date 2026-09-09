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
  (dolist (example '((10 "A" "    A     ") (10 "界面" "   界面   ")
                     (8 "Café" "  Café  ") (8 "abcdef界" " abcdef ")))
    (destructuring-bind (width title expected) example
      (zellij-frame-test-check (string= (zellij-frame-title-line width title) expected)
                               "Centred header mismatch for ~S" title)))
  (let ((spans (zellij-frame-spans '(:rect (0 0 40 24) :title "A" :focus t :mode :normal))))
    (zellij-frame-test-check (= 40 (zellij-frame-string-width (zellij-frame-test-top spans)))
                             "Header must fill the pane width")
    (zellij-frame-test-check (equal (zellij-frame-test-sgr spans) '(0 1 38 5 154 49 7))
                             "Header must use its border colour as background")
    (zellij-frame-test-check (equal (getf (second spans) :sgr) '(0 1 38 5 154 49))
                             "Side borders must retain their foreground colour")
    (zellij-frame-test-check (= 4 (length spans)) "Frame requires four compact spans")
    (zellij-frame-test-check (= 23 (getf (first (last spans)) :y)) "Bottom border coordinate")
    (zellij-frame-test-check
     (handler-case (progn (zellij-frame-title-line 20 (format nil "bad~C" #\Newline)) nil)
       (error () t)) "Control title unexpectedly accepted"))
  (run-zellij-pane-tests)
  (format t "Pane header and frame tests passed~%")
  t)
