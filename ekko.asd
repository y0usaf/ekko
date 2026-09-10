(asdf:defsystem "ekko"
  :description "Ekko terminal multiplexer build spine"
  :version "0.1.0"
  :depends-on ("ekko/core" "ekko/builtins")
  :in-order-to ((test-op (test-op "ekko/tests"))))

(asdf:defsystem "ekko/core"
  :depends-on ("sb-posix" "ekko/runtime")
  :serial t
  :components ((:file "src/package")
               (:file "src/cli")))

(asdf:defsystem "ekko/runtime"
  :depends-on ("sb-posix" "ekko/scene" "ekko/client" "ekko/text" "ekko/extensions")
  :serial t
  :components ((:file "src/platform") (:file "src/assets") (:file "src/vt") (:file "src/history")
               (:file "src/layout") (:file "src/graphics")
               (:file "src/wire") (:file "src/worker") (:file "src/store") (:file "src/server") (:file "src/commands") (:file "src/menus") (:file "src/animations") (:file "src/windows") (:file "src/selection") (:file "src/client")))

(asdf:defsystem "ekko/extensions"
  :depends-on ("ekko/text")
  :components ((:file "src/extensions")))
(asdf:defsystem "ekko/text"
  :components ((:file "src/text-width")))
(asdf:defsystem "ekko/builtins"
  :depends-on ("ekko/extensions")
  :serial t
  :components ((:file "examples/profiles/zellij-pane")
               (:file "examples/profiles/zellij-bindings")
               (:file "examples/profiles/desktop-style") (:file "src/builtins")))

(asdf:defsystem "ekko/scene"
  :description "Pure clipping and rational source transforms"
  :components ((:file "src/geometry")))

(asdf:defsystem "ekko/client"
  :description "Attachment ownership and bounded presentation transactions"
  :components ((:file "src/presentation")))

(asdf:defsystem "ekko/graphics-demo"
  :description "Synthetic Kitty graphics experiment; not a multiplexer"
  :depends-on ("ekko/scene" "ekko/client")
  :components ((:file "src/graphics-demo")))

(asdf:defsystem "ekko/tests"
  :depends-on ("ekko" "ekko/scene" "ekko/client" "ekko/graphics-demo")
  :serial t
  :components ((:file "examples/profiles/zellij-frames")
               (:file "examples/profiles/zellij-pane")
               (:file "tests/zellij-frames")
               (:file "tests/erase-history")
               (:file "tests/geometry")
               (:file "tests/presentation")
               (:file "tests/graphics-demo")
               (:file "tests/input")
               (:file "tests/render")
               (:file "tests/selection")
               (:file "tests/desktop")
               (:file "tests/base64")
               (:file "tests/graphics-parser")
               (:file "tests/assets")
               (:file "tests/customization")
               (:file "tests/runner"))
  :perform (test-op (operation system)
             (declare (ignore operation system))
             (unless (uiop:symbol-call :cl-user :run-ekko-tests)
               (error "Ekko tests failed"))))
