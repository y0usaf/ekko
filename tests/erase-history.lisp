(in-package #:cl-user)

(defun erase-history-test-check (condition format-control &rest arguments)
  (unless condition
    (apply #'error format-control arguments)))

(defun erase-history-test-feed (vt text)
  (ekko/vt:feed vt (ekko/platform:text-bytes text)
                (lambda (kind value) (declare (ignore kind value)))))

(defun erase-history-test-count (vt)
  (if (ekko/vt:terminal-history vt)
      (ekko/vt:history-count (ekko/vt:terminal-history vt))
      0))

(defun run-erase-history-tests ()
  ;; ED2 retains the old behavior unless the terminal policy is enabled.
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3)))
    (erase-history-test-feed vt (format nil "~C[2J" #\Esc))
    (erase-history-test-check (= 0 (erase-history-test-count vt))
                              "default ED2 unexpectedly retained history"))
  ;; The initial grid has one materialized row. Full erase transfers it and
  ;; then materializes the complete viewport, including blank rows.
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "~C[2J" #\Esc))
    (erase-history-test-check (= 1 (erase-history-test-count vt))
                              "fresh ED2 count mismatch: ~D"
                              (erase-history-test-count vt))
    (erase-history-test-check (= 3 (ekko/vt:terminal-materialized-rows vt))
                              "ED2 materialized row count mismatch"))
  ;; Cursor/newline growth counts blank rows as materialized, matching the
  ;; sparse Zellij Grid rather than counting only nonblank cells.
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 4 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "~C[4H~C[2J" #\Esc #\Esc))
    (erase-history-test-check (= 4 (erase-history-test-count vt))
                              "blank-row ED2 count mismatch: ~D"
                              (erase-history-test-count vt)))
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "~C[1;31mX~C[2J" #\Esc #\Esc))
    (erase-history-test-check
     (every (lambda (cell) (equal (second cell) '(0)))
            (subseq (ekko/vt:terminal-cells vt) 0 4))
     "ED2 replacement retained foreground attributes"))
  ;; Standard, bright, indexed, and truecolor backgrounds are all inherited;
  ;; bold and foreground attributes are discarded by the replacement cells.
  (dolist (case-data
            (list
             (list "~C[1;31;44mX~C[2J" '(0 44))
             (list "~C[1;91;104mX~C[2J" '(0 104))
             (list "~C[1;38;5;196;48;5;17mX~C[2J" '(0 48 5 17))
             (list "~C[1;38;5;196;48;2;1;2;3mX~C[2J" '(0 48 2 1 2 3))
             ;; Foreground extended-color payloads can contain values that
             ;; look like standard backgrounds or the 48 introducer.
             (list "~C[38;2;44;48;104mX~C[2J" '(0))
             (list "~C[48;2;101;44;48mX~C[2J" '(0 48 2 101 44 48))))
    (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3 :erase-display-history t)))
      (erase-history-test-feed
       vt (apply #'format nil (first case-data) (list #\Esc #\Esc)))
      (erase-history-test-check
       (every (lambda (cell) (equal (second cell) (second case-data)))
              (subseq (ekko/vt:terminal-cells vt) 0 4))
       "ED2 background inheritance mismatch for ~A" (first case-data))))
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 4 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "A~C~C~C~C~C~C~C[2J"
                                        #\Return #\Linefeed #\Return #\Linefeed
                                        #\Return #\Linefeed #\Esc))
    (erase-history-test-check (= 4 (erase-history-test-count vt))
                              "printed-row ED2 count mismatch: ~D"
                              (erase-history-test-count vt))
    (erase-history-test-feed vt (format nil "~C[2J" #\Esc))
    (erase-history-test-check (= 8 (erase-history-test-count vt))
                              "repeated ED2 count mismatch: ~D"
                              (erase-history-test-count vt)))
  ;; ED0 and ED1 clear in place; ED3 clears existing history.
  (dolist (erase (list "0" "1"))
    (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3 :erase-display-history t)))
      (erase-history-test-feed vt (format nil "~C[2J~C[~AJ" #\Esc #\Esc erase))
      (erase-history-test-check (= 1 (erase-history-test-count vt))
                                "ED~A changed history count" erase)))
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "~C[2J~C[3J" #\Esc #\Esc))
    (erase-history-test-check (= 0 (erase-history-test-count vt))
                              "ED3 did not clear history"))
  ;; Alternate-screen ED2 never transfers main-screen rows.
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 3 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "~C[2J~C[?1049h~C[2J~C[?1049l"
                                        #\Esc #\Esc #\Esc #\Esc))
    (erase-history-test-check (= 1 (erase-history-test-count vt))
                              "alternate ED2 changed main history")
    (erase-history-test-feed vt (format nil "~C[?1049h~C[3J~C[?1049l"
                                        #\Esc #\Esc #\Esc))
    (erase-history-test-check (= 1 (erase-history-test-count vt))
                              "alternate ED3 changed main history"))
  ;; Shrinking a terminal clamps the materialized count before ED2 transfers.
  (let ((vt (ekko/vt:make-terminal :cols 4 :rows 4 :erase-display-history t)))
    (erase-history-test-feed vt (format nil "~C[4H" #\Esc))
    (ekko/vt:resize-terminal vt 4 2 8 16)
    (erase-history-test-feed vt (format nil "~C[2J" #\Esc))
    (erase-history-test-check (= 2 (erase-history-test-count vt))
                              "resized ED2 count mismatch: ~D"
                              (erase-history-test-count vt)))
  (format t "ED2 history tests passed~%")
  t)
