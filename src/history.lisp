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
(defun clear-history (vt)
  "Discard all rows above the main-screen viewport, as ED3 requires."
  (when (and (terminal-history vt) (eq (terminal-screen vt) :main))
    (setf (history-rows (terminal-history vt))
          (make-array +history-rows+ :initial-element nil)
          (history-start (terminal-history vt)) 0
          (history-count (terminal-history vt)) 0
          (history-bytes (terminal-history vt)) 0)))
(defun history-cells (vt)
  "Frozen rows retain the VT's immutable text/rendition cells."
  (let ((history (terminal-history vt)) (cols (terminal-cols vt)))
    (coerce
      (append
        (when (and history (eq (terminal-screen vt) :main))
          (loop for i below (history-count history)
                collect (aref (history-rows history) (mod (+ (history-start history) i) +history-rows+))))
        (loop for y below (terminal-rows vt)
              collect (subseq (terminal-cells vt) (* y cols) (* (1+ y) cols))))
      'vector)))
(defun history-text (vt)
  (map 'vector #'row-text (history-cells vt)))
(defun row-text (row)
  (string-right-trim " " (with-output-to-string (out)
                           (loop for cell across row do (write-string (first cell) out)))))
(export '(terminal-history history-count history-cells history-text clear-history))
