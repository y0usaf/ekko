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
    (if (zellij-frame-hidden-p snapshot)
        (list (ekko/extensions:action
               :decorate :spans (zellij-frame-boundary-spans snapshot)))
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
                       :focus (eql (getf pane :id) focus)
                       :mode mode)))))))))

(ekko/extensions:unregister-component :defaults)
(ekko/extensions:register-component
 :id :zellij-decoration
 :reads '(:focus :mode :panes :viewport :zoom :pane-notes :component-state)
 :handler #'zellij-decoration-hook)
(ekko/extensions:set-option :component :zellij-decoration :name :pane-insets :value '(1 1 1 1))
(ekko/extensions:set-option :component :zellij-decoration :name :viewport-insets :value '(1 0 1 0))
(ekko/extensions:set-option :component :zellij-decoration :name :split-gaps :value '(0 0))
(ekko/extensions:set-option :component :zellij-decoration :name :erase-display-history :value t)
(ekko/extensions:set-option :component :zellij-decoration :name :pty-pixel-source :value :reported)
(ekko/extensions:set-option :component :zellij-decoration :name :viewer-exit-text :value "Bye from Zellij!")
(ekko/extensions:register-component
 :id :zellij-frames :reads '(:component-state)
 :handler (lambda (snapshot event) (declare (ignore snapshot event)) nil))
(ekko/extensions:register-command :component :zellij-frames :name "toggle-frames"
  :handler (lambda (snapshot event)
             (declare (ignore event))
             (let ((hidden (zellij-frame-hidden-p snapshot)))
               (list (ekko/extensions:action
                      :set-geometry
                      :value (unless hidden
                               '(:pane-insets (0 1 1 0)
                                 :boundary-insets (0 0 0 0)
                                 :split-gaps (0 0))))
                     (ekko/extensions:action :set-keymap :name :normal)
                     (ekko/extensions:action
                      :set-state :value (unless hidden '(:hidden t)))))))
(ekko/extensions:register-component
 :id :zellij-modes :reads '(:mode :focus :panes :viewport :zoom :layout :component-state))
(ekko/extensions:bind-key :component :zellij-frames :map :pane
                           :key "z" :command "toggle-frames")
(load (merge-pathnames "zellij-bindings.lisp"
                       (or *load-truename* *default-pathname-defaults*)))
(install-pane-bindings :zellij-modes)

(load (merge-pathnames "zellij-bars.lisp"
                       (or *load-truename* *default-pathname-defaults*)))
(ekko/extensions:register-component
 :id :zellij-bars :reads '(:session :viewport :focus :panes :mode)
 :handler #'zellij-bars-hook)
