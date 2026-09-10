;; Ekko desktop: window headers and a fixed, clickable task dock.
(in-package #:cl-user)
(load (merge-pathnames "zellij.lisp" (or *load-truename* *default-pathname-defaults*)))

(load (merge-pathnames "desktop-style.lisp" (or *load-truename* *default-pathname-defaults*)))
(in-package #:cl-user)

(ekko/extensions:unregister-component :zellij-decoration)
(ekko/extensions:unregister-component :zellij-bars)
(ekko/extensions:unregister-component :zellij-frames)
(ekko/extensions:register-component :id :desktop-windows :reads '(:session :focus :mode :panes :zoom :component-state :viewport :time)
                                    :handler #'ekko/desktop:windows)
(ekko/extensions:set-option :component :desktop-windows :name :pane-insets :value '(1 1 1 1))
(ekko/extensions:set-option :component :desktop-windows :name :viewport-insets :value '(0 0 1 0))
(ekko/extensions:set-option :component :desktop-windows :name :split-gaps :value '(0 0))
(ekko/extensions:set-option :component :desktop-windows :name :erase-display-history :value t)
(ekko/extensions:set-option :component :desktop-windows :name :pty-pixel-source :value :reported)
(ekko/extensions:register-component :id :desktop-dock :reads '(:session :viewport :focus :panes :mode :zoom :time)
                                    :handler #'ekko/desktop:dock)

(ekko/desktop:install-controls :desktop-windows)
