// Package server 提供对外 HTTP 接口。
//
// 接入成本直接决定 SaaS 的销售阻力，所以对主 App 只暴露两个必接接口：
//
//	POST /v1/claims/redeem       核销领奖码（必接）
//	POST /v1/postback/purchase   变现回传（可选，MVP 可后接）
//
// 认证用 API Key + HMAC 签名而非 OAuth —— 少一轮授权流程，就少一周的客户排期。
package server

import (
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/conf"
	"github.com/shaoxiaoxu/linksprout/internal/dao"
	"github.com/shaoxiaoxu/linksprout/internal/service/attribution"
)

// Server 聚合依赖。
type Server struct {
	cfg  *conf.Config
	db   *dao.DB
	attr *attribution.Service
	log  *slog.Logger
}

func New(cfg *conf.Config, db *dao.DB, attr *attribution.Service, log *slog.Logger) *Server {
	return &Server{cfg: cfg, db: db, attr: attr, log: log}
}

// Handler 构造路由。用标准库 ServeMux（Go 1.22+ 支持方法与路径模式），
// MVP 阶段没有引入第三方路由的理由。
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", s.handleHealth)
	mux.HandleFunc("POST /v1/claims/redeem", s.handleRedeem)
	return s.withRecover(s.withRequestLog(mux))
}

func (s *Server) Run(ctx context.Context) error {
	srv := &http.Server{
		Addr:         s.cfg.HTTP.Addr,
		Handler:      s.Handler(),
		ReadTimeout:  s.cfg.HTTP.ReadTimeout,
		WriteTimeout: s.cfg.HTTP.WriteTimeout,
	}
	go func() {
		<-ctx.Done()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdownCtx)
	}()
	s.log.Info("http server listening", "addr", s.cfg.HTTP.Addr)
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}

// ---------------------------------------------------------------- 中间件

func (s *Server) withRecover(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer func() {
			if v := recover(); v != nil {
				s.log.Error("panic", "value", v, "path", r.URL.Path)
				writeErr(w, http.StatusInternalServerError, "internal_error", "内部错误", false)
			}
		}()
		next.ServeHTTP(w, r)
	})
}

func (s *Server) withRequestLog(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		rec := &statusRecorder{ResponseWriter: w, status: http.StatusOK}
		next.ServeHTTP(rec, r)
		s.log.Info("request",
			"method", r.Method, "path", r.URL.Path,
			"status", rec.status, "elapsed_ms", time.Since(start).Milliseconds())
	})
}

type statusRecorder struct {
	http.ResponseWriter
	status int
}

func (r *statusRecorder) WriteHeader(code int) {
	r.status = code
	r.ResponseWriter.WriteHeader(code)
}

// ---------------------------------------------------------------- 响应

// errorBody 的 retryable 字段是刻意设计的：客户需要知道一个错误该不该重试。
// 不给这个信号，客户端要么无脑重试（放大故障），要么无脑放弃（丢收入）。
type errorBody struct {
	Error struct {
		Code      string `json:"code"`
		Message   string `json:"message"`
		Retryable bool   `json:"retryable"`
	} `json:"error"`
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, status int, code, msg string, retryable bool) {
	var b errorBody
	b.Error.Code = code
	b.Error.Message = msg
	b.Error.Retryable = retryable
	writeJSON(w, status, b)
}

func clientIP(r *http.Request) string {
	// 生产环境应改为只信任已知反代注入的头，否则客户端可以伪造 IP 绕过
	// 基于 IP 的风控限流。
	if v := r.Header.Get("X-Forwarded-For"); v != "" {
		if host, _, err := net.SplitHostPort(v); err == nil {
			return host
		}
		return firstToken(v)
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return ""
	}
	return host
}

func firstToken(s string) string {
	for i := 0; i < len(s); i++ {
		if s[i] == ',' || s[i] == ' ' {
			return s[:i]
		}
	}
	return s
}
