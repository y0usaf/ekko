(in-package #:ekko/vt)

;; Bounded main-screen rows. Cells/renditions are replaced by the VT, never
;; mutated in place, so copying the row vector detaches it from screen scrolling.
(defconstant +history-rows+ 10000)
(defconstant +history-bytes+ (* 8 1024 1024))
(defstruct history (rows (make-array +history-rows+ :initial-element nil))
  (start 0) (count 0) (bytes 0))
(defun row-cost (row)
  (loop for cell across row sum (+ 24 (* 4 (length (first cell))) (* 8 (length (second cell))))))
(defun history-drop (history)
  (let* ((index (history-start history)) (row (aref (history-rows history) index)))
    (decf (history-bytes history) (row-cost row))
    (setf (aref (history-rows history) index) nil
          (history-start history) (mod (1+ index) +history-rows+))
    (decf (history-count history))))
(defun remember-row (vt row)
  (let ((cost (row-cost row)))
    (when (> cost +history-bytes+) (return-from remember-row))
    (let ((history (or (terminal-history vt) (setf (terminal-history vt) (make-history)))))
      (loop while (or (= (history-count history) +history-rows+)
                      (> (+ (history-bytes history) cost) +history-bytes+)) do (history-drop history))
      (setf (aref (history-rows history) (mod (+ (history-start history) (history-count history)) +history-rows+)) row)
      (incf (history-count history)) (incf (history-bytes history) cost))))
(defun history-text (vt)
  "Detached text for copy mode. No image bytes or host terminal controls."
  (let ((history (terminal-history vt)) (cols (terminal-cols vt)))
    (coerce
      (append
        (when (and history (eq (terminal-screen vt) :main))
          (loop for i below (history-count history)
                collect (row-text (aref (history-rows history) (mod (+ (history-start history) i) +history-rows+)))))
        (loop for y below (terminal-rows vt)
              collect (row-text (subseq (terminal-cells vt) (* y cols) (* (1+ y) cols)))))
      'vector)))
(defun row-text (row)
  (string-right-trim " " (with-output-to-string (out)
                           (loop for cell across row do (write-string (first cell) out)))))
(export '(terminal-history history-count history-text))
