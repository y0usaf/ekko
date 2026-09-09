;; Ekko's usable session/pane and mode bars. These are ordinary decorations;
;; they do not claim Zellij tab/plugin parity or create synthetic tabs.
(in-package #:cl-user)

(defparameter *zellij-bar-hints*
  '((:normal ("Ctrl-p" "Pane") ("Ctrl-h" "Move") ("Ctrl-o" "Session")
             ("Ctrl-g" "Lock") ("Drag" "Copy") ("Wheel" "Scroll") ("Ctrl-q" "Quit"))
    (:locked ("Ctrl-g" "Unlock"))
    (:pane ("r/d/n" "New pane") ("h/j/k/l" "Focus") ("f" "Fullscreen")
           ("z" "Frames") ("c" "Rename") ("x" "Close") ("Esc" "Normal"))
    (:move ("h/j/k/l" "Move pane") ("n/Tab" "Next") ("p" "Previous") ("Esc" "Normal"))
    (:rename ("Type" "Rename pane") ("Enter" "Save") ("Esc" "Cancel"))
    (:session ("d" "Detach") ("Ctrl-q" "Quit") ("Esc" "Normal"))))

(defun zellij-bar-safe-text (text)
  (map 'string (lambda (c) (if (or (< (char-code c) 32) (<= 127 (char-code c) 159)) #\Space c))
       (or text "")))

(defun zellij-bar-row (width y segments)
  "Fit styled text by terminal cells; the background also clears old labels."
  (let ((x 0) (spans (list (list :x 0 :y y :text (make-string width :initial-element #\Space)
                                :sgr '(0 38 5 250 48 5 236)))))
    (dolist (segment segments)
      (let ((text (zellij-frame-take-width-from-start (zellij-bar-safe-text (first segment)) (- width x))))
        (when (plusp (length text))
          (push (list :x x :y y :text text :sgr (second segment)) spans)
          (incf x (ekko/extensions:display-width text)))))
    (nreverse spans)))

(defun zellij-bars-spans (snapshot)
  (let* ((viewport (getf snapshot :viewport)) (cols (getf viewport :cols))
         (rows (getf viewport :rows)) (insets (getf viewport :insets))
         (focus (getf snapshot :focus)) (panes (getf snapshot :panes))
         (mode (or (getf snapshot :mode) :normal))
         (active '(0 1 38 5 16 48 5 154))
         (base '(0 38 5 250 48 5 236))
         (dim '(0 38 5 245 48 5 236))
         (key '(0 1 38 5 154 48 5 236)))
    (append
     (when (plusp (or (first insets) 0))
       (zellij-bar-row cols 0
         (append (list (list " EKKO " active)
                       (list (format nil " ~A " (zellij-frame-take-width-from-start
                                                 (zellij-bar-safe-text (getf snapshot :session))
                                                 (min 24 (max 0 (- cols 24))))) base)
                       (list " Panes " dim))
                 (loop for pane in panes
                       collect (list (format nil " ~D: ~A " (getf pane :id)
                                             (zellij-frame-take-width-from-start
                                              (zellij-bar-safe-text (zellij-pane-title pane)) 22))
                                     (if (eql (getf pane :id) focus) active dim))))))
     (when (plusp (or (third insets) 0))
       (zellij-bar-row cols (1- rows)
         (cons (list (format nil " ~A " (string-upcase (symbol-name mode)))
                     (if (eq mode :normal) active '(0 1 38 5 16 48 5 166)))
               (loop for (binding label) in (cdr (assoc mode *zellij-bar-hints*))
                     append (list (list (format nil " ~A " binding) key)
                                  (list (concatenate 'string label " ") base)))))))))

(defun zellij-bars-hook (snapshot event)
  (declare (ignore event))
  (list (ekko/extensions:action :decorate :spans
           (zellij-bars-spans
            (loop for key in '(:session :viewport :focus :panes :mode)
                  append (list key (ekko/extensions:value snapshot key)))))))
