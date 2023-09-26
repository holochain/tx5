package main

/*
#include <stdint.h>
#include <stdlib.h>

static inline uintptr_t void_star_to_ptr_t(void *ptr) {
  return (uintptr_t)ptr;
}

static inline void *ptr_to_void_star(uintptr_t ptr) {
  return (void *)ptr;
}

typedef void (*MessageCb) (
  void *usr,
  uintptr_t message_type,
  uintptr_t slot_a,
  uintptr_t slot_b,
  uintptr_t slot_c,
  uintptr_t slot_d
);

static inline void MessageCbInvoke(
  MessageCb cb,
  void *usr,
  uintptr_t message_type,
  uintptr_t slot_a,
  uintptr_t slot_b,
  uintptr_t slot_c,
  uintptr_t slot_d
) {
  cb(usr, message_type, slot_a, slot_b, slot_c, slot_d);
}
*/
import "C"

import (
	"bytes"
	"encoding/json"
	"fmt"
	"sync"
	"unsafe"

	"runtime/debug"

	"github.com/pion/logging"
	"github.com/pion/webrtc/v3"
)

type UintPtrT = C.uintptr_t
type MessageCb = C.MessageCb

func VoidStarToPtrT(v unsafe.Pointer) UintPtrT {
	return C.void_star_to_ptr_t(v)
}

func PtrToVoidStar(ptr UintPtrT) unsafe.Pointer {
	return C.ptr_to_void_star(ptr)
}

func PtrToCharStar(ptr UintPtrT) *byte {
	return (*byte)(C.ptr_to_void_star(ptr))
}

func LoadBytesSafe(ptr UintPtrT, length UintPtrT) *bytes.Buffer {
	raw := unsafe.Slice(PtrToCharStar(ptr), length)

	buf := new(bytes.Buffer)
	buf.Grow(len(raw))
	buf.Write(raw)
	return buf
}

type customLogger struct{}

func (c customLogger) Trace(msg string) {
	EmitTrace(LvlTrace, msg)
}
func (c customLogger) Tracef(format string, args ...interface{}) {
	c.Trace(fmt.Sprintf(format, args...))
}
func (c customLogger) Debug(msg string) {
	EmitTrace(LvlDebug, msg)
}
func (c customLogger) Debugf(format string, args ...interface{}) {
	c.Debug(fmt.Sprintf(format, args...))
}
func (c customLogger) Info(msg string) {
	EmitTrace(LvlInfo, msg)
}
func (c customLogger) Infof(format string, args ...interface{}) {
	c.Info(fmt.Sprintf(format, args...))
}
func (c customLogger) Warn(msg string) {
	EmitTrace(LvlWarn, msg)
}
func (c customLogger) Warnf(format string, args ...interface{}) {
	c.Warn(fmt.Sprintf(format, args...))
}
func (c customLogger) Error(msg string) {
	EmitTrace(LvlError, msg)
}
func (c customLogger) Errorf(format string, args ...interface{}) {
	c.Error(fmt.Sprintf(format, args...))
}

type customLoggerFactory struct{}

func (c customLoggerFactory) NewLogger(subsystem string) logging.LeveledLogger {
	return customLogger{}
}

var webrtcApiMu sync.Mutex
var webrtcApi *webrtc.API

func setWebrtcApi(api *webrtc.API) {
	if api == nil {
		panic("CannotSetWebrtcApiToNil")
	}
	webrtcApiMu.Lock()
	defer webrtcApiMu.Unlock()
	if webrtcApi != nil {
		panic("CannotSetWebrtcApiMultipleTimes")
	}
	webrtcApi = api
}

func getWebrtcApi() *webrtc.API {
	webrtcApiMu.Lock()
	defer webrtcApiMu.Unlock()
	if webrtcApi == nil {
		panic("WebrtcApiIsUnset:CallTx5Init")
	}
	return webrtcApi
}

func MessageCbInvoke(
	cb MessageCb,
	usr unsafe.Pointer,
	message_type UintPtrT,
	slot_a UintPtrT,
	slot_b UintPtrT,
	slot_c UintPtrT,
	slot_d UintPtrT,
) {
	C.MessageCbInvoke(cb, usr, message_type, slot_a, slot_b, slot_c, slot_d)
}

type eventReg struct {
	mu        sync.Mutex
	event_cb  C.MessageCb
	event_usr unsafe.Pointer
}

var globalEventReg eventReg

func EmitEvent(
	message_type UintPtrT,
	slot_a UintPtrT,
	slot_b UintPtrT,
	slot_c UintPtrT,
	slot_d UintPtrT,
) {
	globalEventReg.mu.Lock()
	defer globalEventReg.mu.Unlock()

	if globalEventReg.event_cb == nil {
		// TODO!!! MEMORY LEAK
		// if there is not an event handler
		// we need to clean up (Free) any types in this event
		// that have handles, such as DataChannels and Buffers
		return
	}

	C.MessageCbInvoke(
		globalEventReg.event_cb,
		globalEventReg.event_usr,
		message_type,
		slot_a,
		slot_b,
		slot_c,
		slot_d,
	)
}

type Tx5InitConfig struct {
	EphemeralUdpPortMin *uint16 `json:"ephemeralUdpPortMin,omitempty"`
	EphemeralUdpPortMax *uint16 `json:"ephemeralUdpPortMax,omitempty"`
}

// Initialize the library with some optional configuration.
// You MUST call this exactly ONCE before opening any peer connections.
func CallTx5Init(
	config_buf_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	buf := BufferFromPtr(config_buf_id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	var tmpConfig Tx5InitConfig
	if err := json.Unmarshal(buf.buf.Bytes(), &tmpConfig); err != nil {
		errStr := fmt.Sprintf("%s: %s", err, buf.buf.Bytes())
		panic(errStr)
	}

	setting_engine := webrtc.SettingEngine{
		LoggerFactory: customLoggerFactory{},
	}

	var port_min uint16 = 1
	var port_max uint16 = 65535

	if tmpConfig.EphemeralUdpPortMin != nil {
		port_min = *tmpConfig.EphemeralUdpPortMin
	}

	if tmpConfig.EphemeralUdpPortMax != nil {
		port_max = *tmpConfig.EphemeralUdpPortMax
	}

	setting_engine.SetEphemeralUDPPortRange(port_min, port_max)

	setWebrtcApi(webrtc.NewAPI(webrtc.WithSettingEngine(setting_engine)))

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyTx5Init,
		0,
		0,
		0,
		0,
	)
}

// register the MessageCb that will be invoked for events
//
//export OnEvent
func OnEvent(
	// the callback to invoke
	event_cb C.MessageCb,

	// the user data to forward to the callback
	event_usr unsafe.Pointer,
) unsafe.Pointer {
	globalEventReg.mu.Lock()
	defer globalEventReg.mu.Unlock()

	prev := globalEventReg.event_usr

	globalEventReg.event_cb = event_cb
	globalEventReg.event_usr = event_usr

	return prev
}

const (
	LvlTrace UintPtrT = 0x01
	LvlDebug UintPtrT = 0x02
	LvlInfo  UintPtrT = 0x03
	LvlWarn  UintPtrT = 0x04
	LvlError UintPtrT = 0x05
)

func EmitTrace(
	lvl UintPtrT,
	msg string,
) {
	buf := []byte(msg)

	EmitEvent(
		TyOnTrace,
		lvl,
		C.void_star_to_ptr_t(unsafe.Pointer(&buf[0])),
		UintPtrT(len(buf)),
		0,
	)
}

// make a call into the library
//
//export Call
func Call(
	// call type indicator
	call_type UintPtrT,

	// input params
	slot_a UintPtrT,
	slot_b UintPtrT,
	slot_c UintPtrT,
	slot_d UintPtrT,

	// immediate result callback / response to this call
	// this will always be invoked. nil will panic
	response_cb C.MessageCb,

	// the user data to forward to the result callback
	response_usr unsafe.Pointer,
) {
	// on panics, return error text
	defer func() {
		if err := recover(); err != nil {
			if response_cb == nil {
				return
			}

			bytes := ([]byte)(fmt.Sprintf("%s %s", err, string(debug.Stack())))
			C.MessageCbInvoke(
				response_cb,
				response_usr,
				TyErr,
				// error id ptr
				C.void_star_to_ptr_t(unsafe.Pointer(&([]byte)("Error")[0])),
				// error id len
				UintPtrT(5),
				// error info ptr
				C.void_star_to_ptr_t(unsafe.Pointer(&bytes[0])),
				// error info len
				UintPtrT(len(bytes)),
			)
		}
	}()

	// now that our panic handler is set up, delegate to callInner
	callInner(call_type, slot_a, slot_b, slot_c, slot_d, response_cb, response_usr)
}

func callInner(
	// call type indicator
	call_type UintPtrT,

	// input params
	slot_a UintPtrT,
	slot_b UintPtrT,
	slot_c UintPtrT,
	slot_d UintPtrT,

	// immediate result callback / response to this call
	// this will always be invoked. nil will panic
	response_cb C.MessageCb,

	// the user data to forward to the result callback
	response_usr unsafe.Pointer,
) {
	// -- these calls can be made even if there is no callback pointer -- //

	switch call_type {
	case TyBufferFree:
		CallBufferFree(slot_a)
		return
	case TyPeerConFree:
		CallPeerConFree(slot_a)
		return
	case TyDataChanFree:
		CallDataChanFree(slot_a)
		return
	}

	// -- the remaining calls require response_cb to be non-nil -- //

	if response_cb == nil {
		panic(fmt.Errorf("response_cb cannot be nil for call_type: %d", call_type))
	}

	switch call_type {
	case TyTx5Init:
		CallTx5Init(slot_a, response_cb, response_usr)
	case TyBufferAlloc:
		CallBufferAlloc(response_cb, response_usr)
	case TyBufferAccess:
		CallBufferAccess(slot_a, response_cb, response_usr)
	case TyBufferReserve:
		CallBufferReserve(slot_a, slot_b, response_cb, response_usr)
	case TyBufferExtend:
		CallBufferExtend(slot_a, slot_b, slot_c, response_cb, response_usr)
	case TyBufferRead:
		CallBufferRead(slot_a, slot_b, response_cb, response_usr)
	case TyPeerConAlloc:
		CallPeerConAlloc(slot_a, response_cb, response_usr)
	case TyPeerConStats:
		CallPeerConStats(slot_a, response_cb, response_usr)
	case TyPeerConCreateOffer:
		CallPeerConCreateOffer(slot_a, slot_b, response_cb, response_usr)
	case TyPeerConCreateAnswer:
		CallPeerConCreateAnswer(slot_a, slot_b, response_cb, response_usr)
	case TyPeerConSetLocalDesc:
		CallPeerConSetLocalDesc(slot_a, slot_b, response_cb, response_usr)
	case TyPeerConSetRemDesc:
		CallPeerConSetRemDesc(slot_a, slot_b, response_cb, response_usr)
	case TyPeerConAddICECandidate:
		CallPeerConAddICECandidate(slot_a, slot_b, response_cb, response_usr)
	case TyPeerConCreateDataChan:
		CallPeerConCreateDataChan(slot_a, slot_b, response_cb, response_usr)
	case TyDataChanLabel:
		CallDataChanLabel(slot_a, response_cb, response_usr)
	case TyDataChanReadyState:
		CallDataChanReadyState(slot_a, response_cb, response_usr)
	case TyDataChanSend:
		CallDataChanSend(slot_a, slot_b, response_cb, response_usr)
	case TyDataChanSetBufferedAmountLowThreshold:
		CallDataChanSetBufferedAmountLowThreshold(slot_a, slot_b, response_cb, response_usr)
	case TyDataChanBufferedAmount:
		CallDataChanBufferedAmount(slot_a, response_cb, response_usr)
	default:
		panic(fmt.Errorf("invalid call_type: %d", call_type))
	}
}

func main() {}
