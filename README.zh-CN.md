# phira-mp-plus

`phira-mp-plus` 是基于 [TeamFlos/phira-mp](https://github.com/TeamFlos/phira-mp)(Apache-2.0)修改的衍生版本,新增了服务端管理功能。归属说明与改动清单见 [NOTICE](NOTICE)。

本项目与 TeamFlos 无隶属关系,亦未获得其背书。

`phira-mp` 是一个用 Rust 开发的项目。 以下是部署和运行该项目服务端的步骤。

简体中文 | [English Version](README.md)

## 本仓库新增

- 终端(TUI)控制台:仪表盘、会话、房间、网络、IP访问控制、流量限制、服务器、日志
- IP 黑白名单,支持 glob(`*`、`?`、`[...]`)与 CIDR 匹配,可热重载
- 按 IP 限速,超限动作可选(告警 / 断开 / 封禁)
- 临时维护模式(拒绝新连接并断开现有连接)
- `--ip-version v4|v6|both` 监听地址选择

## 环境

- Rust 1.85 或更高版本

## 服务端安装
### 对于 `Linux` 用户
#### 依赖
首先，如果尚未安装 Rust，请安装。 您可以按照 https://www.rust-lang.org/tools/install 中的说明进行操作

对于 Ubuntu 或 Debian 用户，如果尚未安装“curl”，请使用以下命令进行安装：

```shell
sudo apt install curl
```
对于 Fedora 或 CentOS 用户，请使用以下命令：
```shell
sudo yum install curl
```
安装curl后，使用以下命令安装Rust：
```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
然后，构建项目：
```shell
cargo build --release -p phira-mp-server
```
#### 运行服务端
您可以使用以下命令运行该应用程序：
```shell
RUST_LOG=info target/release/phira-mp-server
```

也可以通过参数指定端口：
```shell
RUST_LOG=info target/release/phira-mp-server --port 8080
```

### For docker

1. 创建 Dockerfile
```
FROM ubuntu:22.04

RUN apt-get update && apt-get -y upgrade && apt-get install -y curl git build-essential pkg-config openssl libssl-dev

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /root/
RUN git clone https://github.com/xlhz/phira-mp-plus
WORKDIR /root/phira-mp-plus
RUN cargo build --release -p phira-mp-server

ENTRYPOINT ["/root/phira-mp-plus/target/release/phira-mp-server", "--port", "<preferred-port>"]
```

2. 构建镜像
`docker build --tag phira-mp .`

3. 运行容器
`docker run -it --name phira-mp -p <prefered-port>:<preferred-port> --restart=unless-stopped phira-mp`

#### 监控
您可以检查正在运行的进程及其正在侦听的端口：
```shell
ps -aux | grep phira-mp-server
netstat -tuln | grep 12346
```
![result](https://i.imgur.com/NXC54ZZ.png)

## 对于 Windows 或 Android 用户
查看: [https://docs.qq.com/doc/DU1dlekx3U096REdD](https://docs.qq.com/doc/DU1dlekx3U096REdD)
