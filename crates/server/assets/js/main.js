up.compiler('canvas.chart', function (canvas, data) {
  // const cfg = data;
  // {
  //   type: data.chartType || 'line',
  //   data: {
  //     datasets: [{
  //       label:            data.label       || 'Series',
  //       data:             JSON.parse(data.points),
  //       borderColor:      data.borderColor || '#38bdf8',
  //       backgroundColor:  data.bgColor     || 'transparent',
  //       tension:      parseFloat(data.tension || 0),
  //       fill:         data.fill === 'true',
  //       parsing:      false
  //     }]
  //   },
  //   options: {
  //     responsive: true,
  //     maintainAspectRatio: false,
  //     scales: {
  //       x: { type: 'time', time: { tooltipFormat: 'yyyy-MM-dd HH:mm' } }
  //     }
  //   }
  // };

  const chart = new Chart(canvas, data);
  return () => chart.destroy();
});

up.compiler("time[datetime]", function(el, data) {
  const date = new Date(el.dateTime);          // parsed as UTC
  el.textContent = date.toLocaleString();      // user’s local TZ & locale
});
