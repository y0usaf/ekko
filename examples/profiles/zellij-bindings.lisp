;; Shared pane, move, rename and session bindings, through the public API.
(in-package #:cl-user)

(defun install-pane-bindings (owner)
(dolist (mode '(:normal :locked :pane :move :rename :session))
  (ekko/extensions:register-keymap :component owner :name mode
                                    :unbound (cond ((eq mode :rename) "rename-input")
                                                   ((member mode '(:pane :move :session)) :ignore)
                                                   (t :forward))))
(ekko/extensions:set-option :component owner :name :initial-keymap :value :normal)
(dolist (spec '((:normal :locked "lock") (:locked :normal "unlock")))
  (destructuring-bind (from to command) spec
    (let ((target to))
      (ekko/extensions:register-command :component owner :name command
        :handler (lambda (snapshot event) (declare (ignore snapshot event))
                   (list (ekko/extensions:action :set-keymap :name target)))))
    (ekko/extensions:bind-key :component owner :map from :key "C-g" :command command)))
(ekko/extensions:register-command :component owner :name "pane-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :pane))))
(ekko/extensions:register-command :component owner :name "normal-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:register-command :component owner :name "pane-fullscreen"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :zoom)
                   (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:bind-key :component owner :map :normal :key "C-p" :command "pane-mode")
(ekko/extensions:register-command :component owner :name "move-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :move))))
(dolist (map '(:normal :pane))
  (ekko/extensions:bind-key :component owner :map map :key "C-h" :command "move-mode"))
(dolist (key '("C-p" "Enter" "Escape"))
  (ekko/extensions:bind-key :component owner :map :pane :key key :command "normal-mode"))
(ekko/extensions:bind-key :component owner :map :pane :key "C-g" :command "lock")
(ekko/extensions:bind-key :component owner :map :pane :key "f" :command "pane-fullscreen")
(ekko/extensions:register-command :component owner :name "rename-enter"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-rename-enter-actions snapshot (string-downcase (string owner)))))
(ekko/extensions:register-command :component owner :name "rename-input"
  :handler (lambda (snapshot event)
             (zellij-pane-rename-input-action snapshot event)))
(ekko/extensions:register-command :component owner :name "rename-previous"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-rename-previous-action snapshot (string-downcase (string owner)))))
(ekko/extensions:bind-key :component owner :map :pane :key "c" :command "rename-enter")
(dolist (key '("Enter" "C-c"))
  (ekko/extensions:bind-key :component owner :map :rename
                             :key key :command "normal-mode"))
(ekko/extensions:bind-key :component owner :map :rename
                           :key "Escape" :command "rename-previous")
(ekko/extensions:bind-key :component owner :map :rename :key "C-g" :command "lock")
(ekko/extensions:bind-key :component owner :map :rename :key "C-p" :command "pane-mode")
(ekko/extensions:bind-key :component owner :map :rename :key "C-h" :command "move-mode")
(dolist (direction '(:left :right :up :down))
  (let ((name (format nil "pane-focus-~(~A~)" direction))
        (direction direction))
    (ekko/extensions:register-command :component owner :name name
      :handler (lambda (snapshot event) (declare (ignore event))
                 (zellij-pane-focus-action snapshot direction)))
    (dolist (key (case direction
                   (:left '("h" "Left")) (:right '("l" "Right"))
                   (:up '("k" "Up")) (:down '("j" "Down"))))
      (ekko/extensions:bind-key :component owner :map :pane
                                 :key key :command name))))
(ekko/extensions:register-command :component owner :name "pane-switch-focus"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-switch-focus-action snapshot)))
(ekko/extensions:bind-key :component owner :map :pane :key "p" :command "pane-switch-focus")
(dolist (spec '(("pane-new-down" :rows) ("pane-new-right" :columns)))
  (destructuring-bind (name axis) spec
    (ekko/extensions:register-command :component owner :name name
      :handler (lambda (snapshot event) (declare (ignore event))
                 (zellij-pane-split-actions axis
                                            (getf (zellij-pane-focused snapshot) :id)
                                            snapshot)))))
(ekko/extensions:register-command :component owner :name "pane-new"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-no-preference-actions snapshot)))
(ekko/extensions:bind-key :component owner :map :pane :key "n" :command "pane-new")
(ekko/extensions:bind-key :component owner :map :pane :key "d" :command "pane-new-down")
(ekko/extensions:bind-key :component owner :map :pane :key "r" :command "pane-new-right")
(ekko/extensions:register-command :component owner :name "move-pane-next"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-move-cyclic-action snapshot nil)))
(ekko/extensions:register-command :component owner :name "move-pane-previous"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-move-cyclic-action snapshot t)))
(ekko/extensions:bind-key :component owner :map :move :key "n" :command "move-pane-next")
(ekko/extensions:bind-key :component owner :map :move :key "Tab" :command "move-pane-next")
(ekko/extensions:bind-key :component owner :map :move :key "p" :command "move-pane-previous")
(dolist (spec '(("h" "Left" :left) ("j" "Down" :down)
                ("k" "Up" :up) ("l" "Right" :right)))
  (destructuring-bind (key alias direction) spec
    (let ((name (format nil "move-pane-~(~A~)" direction))
          (direction direction))
      (ekko/extensions:register-command :component owner :name name
        :handler (lambda (snapshot event) (declare (ignore event))
                   (zellij-pane-move-direction-action snapshot direction)))
      (dolist (binding (list key alias))
        (ekko/extensions:bind-key :component owner :map :move
                                   :key binding :command name)))))
(dolist (key '("C-h" "Enter" "Escape"))
  (ekko/extensions:bind-key :component owner :map :move
                             :key key :command "normal-mode"))
(ekko/extensions:bind-key :component owner :map :move :key "C-g" :command "lock")
(ekko/extensions:bind-key :component owner :map :move :key "C-p" :command "pane-mode")
(ekko/extensions:register-command :component owner :name "pane-close"
  :handler (lambda (snapshot event) (declare (ignore event))
             (zellij-pane-close-actions snapshot)))
(ekko/extensions:bind-key :component owner :map :pane :key "x" :command "pane-close")

;; Session policy remains ordinary action-returning Lisp. Plugin launchers in
;; this mode require future public mechanisms and remain in the surface ledger.
(ekko/extensions:register-command :component owner :name "session-mode"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :session))))
(ekko/extensions:register-command :component owner :name "session-quit"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :stop))))
(ekko/extensions:register-command :component owner :name "session-detach"
  :handler (lambda (snapshot event) (declare (ignore snapshot event))
             (list (ekko/extensions:action :detach)
                   (ekko/extensions:action :set-keymap :name :normal))))
(dolist (map '(:normal :pane :move :rename :session))
  (ekko/extensions:bind-key :component owner :map map
                             :key "C-q" :command "session-quit")
  (unless (eq map :session)
    (ekko/extensions:bind-key :component owner :map map
                               :key "C-o" :command "session-mode")))
(dolist (key '("C-o" "Enter" "Escape"))
  (ekko/extensions:bind-key :component owner :map :session
                             :key key :command "normal-mode"))
(dolist (spec '(("C-g" "lock") ("C-p" "pane-mode") ("C-h" "move-mode")
                ("d" "session-detach")))
  (ekko/extensions:bind-key :component owner :map :session
                             :key (first spec) :command (second spec)))

)
