// Package conf 加载运行配置。
package conf

import (
	"fmt"
	"os"
	"time"

	"gopkg.in/yaml.v3"
)

type Config struct {
	HTTP struct {
		Addr         string        `yaml:"addr"`
		ReadTimeout  time.Duration `yaml:"read_timeout"`
		WriteTimeout time.Duration `yaml:"write_timeout"`
	} `yaml:"http"`

	Postgres struct {
		DSN string `yaml:"dsn"`
	} `yaml:"postgres"`

	Redis struct {
		Addr string `yaml:"addr"`
	} `yaml:"redis"`

	Attribution struct {
		PolicyVersion string `yaml:"policy_version"`
	} `yaml:"attribution"`
}

// Load 读取配置文件，并允许用环境变量覆盖敏感项。
//
// DSN 之所以支持环境变量覆盖：配置文件会进版本库，而连接串含密码。
func Load(path string) (*Config, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("conf: 读取 %s 失败: %w", path, err)
	}
	var c Config
	if err := yaml.Unmarshal(b, &c); err != nil {
		return nil, fmt.Errorf("conf: 解析 %s 失败: %w", path, err)
	}
	if v := os.Getenv("LINKSPROUT_PG_DSN"); v != "" {
		c.Postgres.DSN = v
	}
	if v := os.Getenv("LINKSPROUT_REDIS_ADDR"); v != "" {
		c.Redis.Addr = v
	}
	if c.HTTP.Addr == "" {
		c.HTTP.Addr = ":8080"
	}
	if c.HTTP.ReadTimeout == 0 {
		c.HTTP.ReadTimeout = 10 * time.Second
	}
	if c.HTTP.WriteTimeout == 0 {
		c.HTTP.WriteTimeout = 15 * time.Second
	}
	if c.Attribution.PolicyVersion == "" {
		c.Attribution.PolicyVersion = "v1"
	}
	return &c, nil
}
