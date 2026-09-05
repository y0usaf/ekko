(asdf:defsystem "ekko"
  :description "Ekko terminal multiplexer build spine"
  :version "0.1.0"
  :depends-on ("sb-posix" "ekko/runtime")
  :serial t
  :in-order-to ((test-op (test-op "ekko/tests")))
  :components ((:file "src/package")
               (:file "src/cli")))

(asdf:defsystem "ekko/runtime"
  :depends-on ("sb-posix" "ekko/scene" "ekko/client")
  :serial t
  :components ((:file "src/platform") (:file "src/assets") (:file "src/vt") (:file "src/graphics")
               (:file "src/wire") (:file "src/server") (:file "src/client")))

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
  :components ((:file "tests/geometry")
               (:file "tests/presentation")
               (:file "tests/graphics-demo")
               (:file "tests/input")
               (:file "tests/render")
               (:file "tests/base64")
               (:file "tests/graphics-parser")
               (:file "tests/assets")
               (:file "tests/runner"))
  :perform (test-op (operation system)
             (declare (ignore operation system))
             (unless (uiop:symbol-call :cl-user :run-ekko-tests)
               (error "Ekko tests failed"))))
