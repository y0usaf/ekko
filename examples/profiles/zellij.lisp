;; Zellij 0.43.1 compatibility profile, under development.
;; The implemented Pane slice covers mode control, directional/row-major
;; focus, new panes Down/Right/NoPreference, close, and fullscreen. Other Pane
;; actions remain intentionally unregistered. See docs/zellij/README.md.
(in-package #:cl-user)

;; The extension worker may load this source from a stream, so *LOAD-TRUENAME*
;; can be NIL. In that case its default pathname is the configuration
;; directory. The helper is deliberately a sibling of this profile so copied
;; test configurations can remain self-contained.
(load (merge-pathnames "zellij-frames.lisp"
                       (or *load-truename* *default-pathname-defaults*)))
(load (merge-pathnames "zellij-pane.lisp"
                       (or *load-truename* *default-pathname-defaults*)))

(defun zellij-decoration-hook (snapshot event)
  (declare (ignore event))
  (let ((focus (ekko/extensions:value snapshot :focus))
        (mode (ekko/extensions:value snapshot :mode))
        (notes (ekko/extensions:value snapshot :pane-notes)))
    (list
     (ekko/extensions:action
      :decorate :spans
      (loop for pane in (ekko/extensions:value snapshot :panes)
            when (getf pane :visible t)
              append
                (let ((note (find (getf pane :id) notes
                                  :from-end t
                                  :key (lambda (entry) (getf entry :pane)))))
                (zellij-frame-spans
                 (list :rect (getf pane :outer-rect)
                       :title (if note
                                  (getf note :text)
                                  (if (and (eq mode :rename)
                                           (eql (getf pane :id) focus)
                                           (zerop (length (or (getf pane :name) ""))))
                                      "Enter name..."
                                      (zellij-pane-title pane)))
                       :sgr (and note (getf note :sgr))
                       ;; Ekko currently exposes the bounded history count;
                       ;; zero position is honest until below-viewport state is
                       ;; part of the public snapshot.
                       :scroll (list 0 (getf pane :history-rows 0))
                       :focus (eql (getf pane :id) focus)
                       :mode mode))))))))

(ekko/extensions:unregister-component :defaults)
(ekko/extensions:register-component
 :id :zellij-decoration
 :reads '(:focus :mode :panes :viewport :zoom :pane-notes)
 :handler #'zellij-decoration-hook)
(ekko/extensions:set-option :component :zellij-decoration :name :pane-insets :value '(1 1 1 1))
(ekko/extensions:set-option :component :zellij-decoration :name :viewport-insets :value '(1 0 1 0))
(ekko/extensions:set-option :component :zellij-decoration :name :split-gaps :value '(0 0))
(ekko/extensions:set-option :component :zellij-decoration :name :erase-display-history :value t)
(ekko/extensions:set-option :component :zellij-decoration :name :pty-pixel-source :value :reported)
(ekko/extensions:register-component
 :id :zellij-modes :reads '(:mode :focus :panes :viewport :zoom :layout :component-state))
(dolist (mode '(:normal :locked :pane :move :rename))
  (ekko/extensions:register-keymap :component :zellij-modes :name mode
                                    :unbound (cond ((eq mode :rename) "rename-input")
                                                   ((member mode '(:pane :move)) :ignore)
                                                   (t :forward))))
(ekko/extensions:set-option :component :zellij-modes :name :initial-keymap :value :normal)
(dolist (spec '((:normal :locked "lock") (:locked :normal "unlock")))
  (destructuring-bind (from to command) spec
    (let ((target to))
      (ekko/extensions:register-command :component :zellij-modes :name command
        :handler (lambda (snapshot event) (declare (ignore snapshot event))
                   (list (ekko/extensions:action :set-keymap :name target)))))
    (ekko/extensions:bind-key :component :zellij-modes :map from :key "C-g" :command command)))
(ekko/extensions:register-command :component :zellij-modes :name "pane-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :pane))))
(ekko/extensions:register-command :component :zellij-modes :name "normal-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:register-command :component :zellij-modes :name "pane-fullscreen"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :zoom)
                   (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:bind-key :component :zellij-modes :map :normal :key "C-p" :command "pane-mode")
(ekko/extensions:register-command :component :zellij-modes :name "move-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :move))))
(dolist (map '(:normal :pane))
  (ekko/extensions:bind-key :component :zellij-modes :map map :key "C-h" :command "move-mode"))
(dolist (key '("C-p" "Enter" "Escape"))
  (ekko/extensions:bind-key :component :zellij-modes :map :pane :key key :command "normal-mode"))
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "C-g" :command "lock")
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "f" :command "pane-fullscreen")
(ekko/extensions:register-command :component :zellij-modes :name "rename-enter"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-rename-enter-actions snapshot)))
(ekko/extensions:register-command :component :zellij-modes :name "rename-input"
  :handler (lambda (snapshot event)
             (zellij-pane-rename-input-action snapshot event)))
(ekko/extensions:register-command :component :zellij-modes :name "rename-previous"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-rename-previous-action snapshot)))
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "c" :command "rename-enter")
(dolist (key '("Enter" "C-c"))
  (ekko/extensions:bind-key :component :zellij-modes :map :rename
                             :key key :command "normal-mode"))
(ekko/extensions:bind-key :component :zellij-modes :map :rename
                           :key "Escape" :command "rename-previous")
(ekko/extensions:bind-key :component :zellij-modes :map :rename :key "C-g" :command "lock")
(ekko/extensions:bind-key :component :zellij-modes :map :rename :key "C-p" :command "pane-mode")
(ekko/extensions:bind-key :component :zellij-modes :map :rename :key "C-h" :command "move-mode")
(dolist (direction '(:left :right :up :down))
  (let ((name (format nil "pane-focus-~(~A~)" direction))
        (direction direction))
    (ekko/extensions:register-command :component :zellij-modes :name name
      :handler (lambda (snapshot event) (declare (ignore event))
                 (zellij-pane-focus-action snapshot direction)))
    (dolist (key (case direction
                   (:left '("h" "Left")) (:right '("l" "Right"))
                   (:up '("k" "Up")) (:down '("j" "Down"))))
      (ekko/extensions:bind-key :component :zellij-modes :map :pane
                                 :key key :command name))))
(ekko/extensions:register-command :component :zellij-modes :name "pane-switch-focus"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-switch-focus-action snapshot)))
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "p" :command "pane-switch-focus")
(dolist (spec '(("pane-new-down" :rows) ("pane-new-right" :columns)))
  (destructuring-bind (name axis) spec
    (ekko/extensions:register-command :component :zellij-modes :name name
      :handler (lambda (snapshot event) (declare (ignore event))
                 (zellij-pane-split-actions axis
                                            (getf (zellij-pane-focused snapshot) :id)
                                            snapshot)))))
(ekko/extensions:register-command :component :zellij-modes :name "pane-new"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-no-preference-actions snapshot)))
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "n" :command "pane-new")
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "d" :command "pane-new-down")
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "r" :command "pane-new-right")
(ekko/extensions:register-command :component :zellij-modes :name "move-pane-next"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-move-cyclic-action snapshot nil)))
(ekko/extensions:register-command :component :zellij-modes :name "move-pane-previous"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-move-cyclic-action snapshot t)))
(ekko/extensions:bind-key :component :zellij-modes :map :move :key "n" :command "move-pane-next")
(ekko/extensions:bind-key :component :zellij-modes :map :move :key "Tab" :command "move-pane-next")
(ekko/extensions:bind-key :component :zellij-modes :map :move :key "p" :command "move-pane-previous")
(dolist (spec '(("h" "Left" :left) ("j" "Down" :down)
                ("k" "Up" :up) ("l" "Right" :right)))
  (destructuring-bind (key alias direction) spec
    (let ((name (format nil "move-pane-~(~A~)" direction))
          (direction direction))
      (ekko/extensions:register-command :component :zellij-modes :name name
        :handler (lambda (snapshot event) (declare (ignore event))
                   (zellij-pane-move-direction-action snapshot direction)))
      (dolist (binding (list key alias))
        (ekko/extensions:bind-key :component :zellij-modes :map :move
                                   :key binding :command name)))))
(dolist (key '("C-h" "Enter" "Escape"))
  (ekko/extensions:bind-key :component :zellij-modes :map :move
                             :key key :command "normal-mode"))
(ekko/extensions:bind-key :component :zellij-modes :map :move :key "C-g" :command "lock")
(ekko/extensions:bind-key :component :zellij-modes :map :move :key "C-p" :command "pane-mode")
(ekko/extensions:register-command :component :zellij-modes :name "pane-close"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-close-actions snapshot)))
(ekko/extensions:bind-key :component :zellij-modes :map :pane :key "x" :command "pane-close")
