package main

import (
	"fmt"
	"runtime/cgo"
	"sync"
	"unsafe"

	"github.com/pion/webrtc/v3"
)

type DataChan struct {
	mu     sync.Mutex
	closed bool
	ch     *webrtc.DataChannel
	handle UintPtrT
}

// If you invoke this function, you *must* call Free,
// otherwise the channel will be leaked.
func NewDataChan(ch *webrtc.DataChannel) *DataChan {
	dataChan := new(DataChan)
	dataChan.ch = ch

	handle := UintPtrT(cgo.NewHandle(dataChan))
	dataChan.handle = handle

	ch.OnClose(func() {
		EmitEvent(
			TyDataChanOnClose,
			handle,
			0,
			0,
			0,
		)
	})

	ch.OnOpen(func() {
		EmitEvent(
			TyDataChanOnOpen,
			handle,
			0,
			0,
			0,
		)
	})

	ch.OnError(func(err error) {
		fmt.Printf("go DATA CHAN ERR %v\n", err)
	})

	ch.OnMessage(func(msg webrtc.DataChannelMessage) {
		buf := NewBuffer(msg.Data)
		EmitEvent(
			TyDataChanOnMessage,
			handle,
			buf.handle,
			0,
			0,
		)
	})

	return dataChan
}

func (dataChan *DataChan) Free() {
	dataChan.mu.Lock()
	defer dataChan.mu.Unlock()

	if dataChan.closed {
		return
	}

	dataChan.closed = true
	(cgo.Handle)(dataChan.handle).Delete()

	dataChan.ch.Close()
	dataChan.ch = nil
}

func CallDataChanFree(data_chan_id UintPtrT) {
	hnd := cgo.Handle(data_chan_id)
	dataChan := hnd.Value().(*DataChan)
	dataChan.Free()
}

func CallDataChanReadyState(
	data_chan_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(data_chan_id)
	dataChan := hnd.Value().(*DataChan)
	dataChan.mu.Lock()
	defer dataChan.mu.Unlock()

	if dataChan.closed {
		panic("DataChanClosed")
	}

	state := UintPtrT(dataChan.ch.ReadyState())

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyDataChanReadyState,
		state,
		0,
		0,
		0,
	)
}

func CallDataChanSend(
	data_chan_id UintPtrT,
	buffer_id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	hnd := cgo.Handle(data_chan_id)
	dataChan := hnd.Value().(*DataChan)
	dataChan.mu.Lock()
	defer dataChan.mu.Unlock()

	if dataChan.closed {
		panic("DataChanClosed")
	}

	buf_hnd := cgo.Handle(buffer_id)
	buf := buf_hnd.Value().(*Buffer)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	err := dataChan.ch.Send(buf.buf.Bytes())

	buf.FreeAlreadyLocked()

	if err != nil {
		panic(err)
	}

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyDataChanSend,
		0,
		0,
		0,
		0,
	)
}
