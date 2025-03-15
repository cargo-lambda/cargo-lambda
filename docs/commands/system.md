# Cargo Lambda System

The `system` or `config` command gives you information about the current installation of Zig and the configuration of your project.

## Show Configuration

To show the current configuration, run the `system` command:

```sh
cargo lambda system
```

If you're working in a workspace, you can specify the package name to show information for a specific package:

```sh
cargo lambda system --package package-name
```

### Output Format

You can change the output format to `json` or `text`:

```sh
cargo lambda system --output-format json
```

If you don't specify the output format, the default is `text`.

This is how the text output looks like for a project with a single package:

```yaml
zig:
  path: /opt/zig/latest/zig
config: !package
  build: {}
  deploy:
    ipv6_allowed_for_dual_stack: false
  watch:
    invoke_address: '::'
    invoke_port: 9000
```

This is how the text output looks like for a workspace with multiple packages:

```yaml
zig:
  path: /opt/zig/latest/zig
config: !global
  workspace:
    build: {}
    deploy:
      ipv6_allowed_for_dual_stack: false
    watch:
      invoke_address: '::'
      invoke_port: 9000
      router:
      - path: /organizations/{user_id}/offices/{post_id}/prospects
        function: fun1
      - path: /users/{user_id}
        methods:
        - POST
        - PUT
        function: post_user
      - path: /users/{user_id}
        methods:
        - GET
        function: get_user
  packages:
    fun1:
      build: {}
      deploy:
        tag:
        - organization=aws
        - team=lambda
        timeout: 120
        ipv6_allowed_for_dual_stack: false
        env_var:
        - APP_ENV=production
      watch: {}
```


## Install Zig

The system command can also be used to install Zig. To install Zig, run the `system` command with the `--install-zig` flag:

```sh
cargo lambda system --install-zig
```
